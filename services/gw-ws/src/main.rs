#![allow(clippy::result_large_err)]

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use sarvex_auth::{hash_api_key, Authenticator};
use sarvex_contracts::sarvex::v1::BookSide;
use sarvex_me_client::MeCoreClient;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle};

#[derive(Clone)]
struct AppState {
    nats_url: String,
    me_addr: String,
    auth: Arc<Authenticator>,
    pool: sqlx::PgPool,
}

#[derive(Debug, Deserialize)]
struct ClientCommand {
    op: String,
    channel: Option<String>,
    ticker: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WireEnvelope {
    event_id: String,
    global_seq: u64,
    contract_seq: Option<u64>,
    payload: WireFill,
}

#[derive(Debug, Clone, Deserialize)]
struct WireFill {
    ticker: String,
    maker_user_id: String,
    taker_user_id: String,
    maker_order_id: String,
    taker_order_id: String,
    price_ticks: i64,
    count: i64,
    aggressor_side: i32,
}

#[derive(Debug, Clone, Deserialize)]
struct WireBookEnvelope {
    event_id: String,
    global_seq: u64,
    contract_seq: Option<u64>,
    payload: WireBookDelta,
}

#[derive(Debug, Clone, Deserialize)]
struct WireBookDelta {
    ticker: String,
    side: i32,
    book_side: Option<i32>,
    book_seq: Option<u64>,
    price_ticks: i64,
    qty_delta: i64,
    new_total_qty: i64,
    new_order_count: Option<i32>,
}

#[derive(Debug, Clone)]
enum MarketEvent {
    Book(WireBookEnvelope),
    Trade(WireEnvelope),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    sarvex_runtime::init_tracing("gw-ws");
    let auth = Arc::new(Authenticator::from_env()?);
    let pool = sarvex_db::connect().await?;
    let state = AppState {
        nats_url: env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        me_addr: env::var("ME_CORE_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50054".to_owned()),
        auth,
        pool,
    };
    let port = env::var("HTTP_PORT").unwrap_or_else(|_| "8082".to_owned());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .route("/ws", get(ws_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn healthz() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({"status":"ok","service":"gw-ws"})),
    )
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match async_nats::connect(&state.nats_url).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({"status":"ready","service":"gw-ws"})),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status":"not_ready","service":"gw-ws","error":error.to_string()})),
        ),
    }
}
async fn metrics() -> Response {
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], "# HELP sarvex_gateway_up Gateway health state.\n# TYPE sarvex_gateway_up gauge\nsarvex_gateway_up{service=\"gw-ws\"} 1\n").into_response()
}

async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let user_id = authenticated_user(&headers, &state.auth, &state.pool).await;
    upgrade
        .on_upgrade(move |socket| handle_socket(socket, state, user_id))
        .into_response()
}

async fn handle_socket(socket: WebSocket, state: AppState, user_id: Option<String>) {
    let (mut sender, mut receiver) = socket.split();
    let Ok(nats) = async_nats::connect(&state.nats_url).await else {
        let _ = sender
            .send(Message::Text(
                json!({"type":"error","code":"NATS_UNAVAILABLE"})
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    };
    if sender
        .send(Message::Text(
            json!({"type":"connected","service":"gw-ws"})
                .to_string()
                .into(),
        ))
        .await
        .is_err()
    {
        return;
    }
    let (event_tx, mut event_rx) = mpsc::channel::<Value>(128);
    let mut subscriptions: Vec<JoinHandle<()>> = Vec::new();
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                let Some(Ok(message)) = incoming else { break };
                match message {
                    Message::Text(text) => {
                        let command: ClientCommand = match serde_json::from_str(&text) {
                            Ok(value) => value,
                            Err(_) => {
                                if sender.send(Message::Text(json!({"type":"error","code":"INVALID_MESSAGE"}).to_string().into())).await.is_err() { break; }
                                continue;
                            }
                        };
                        if command.op.eq_ignore_ascii_case("subscribe") {
                            let Some(channel) = command.channel else {
                                if sender.send(Message::Text(json!({"type":"error","code":"CHANNEL_REQUIRED"}).to_string().into())).await.is_err() { break; }
                                continue;
                            };
                            let Some(ticker) = command.ticker else {
                                if sender.send(Message::Text(json!({"type":"error","code":"TICKER_REQUIRED"}).to_string().into())).await.is_err() { break; }
                                continue;
                            };
                            if channel.eq_ignore_ascii_case("private") && user_id.is_none() {
                                if sender.send(Message::Text(json!({"type":"error","code":"UNAUTHENTICATED"}).to_string().into())).await.is_err() { break; }
                                continue;
                            }
                            let tx = event_tx.clone();
                            let private_user = if channel.eq_ignore_ascii_case("private") { user_id.clone() } else { None };
                            let nats_client = nats.clone();
                            if channel.eq_ignore_ascii_case("market") {
                                subscriptions.push(tokio::spawn(run_market_subscription(
                                    nats_client, state.me_addr.clone(), ticker.clone(), tx,
                                )));
                                if sender.send(Message::Text(json!({"type":"subscribed","channel":channel,"ticker":ticker}).to_string().into())).await.is_err() { break; }
                            } else {
                                let subject = format!("exec.fills.{ticker}");
                                match nats_client.subscribe(subject.clone()).await {
                                    Ok(mut subscription) => {
                                        subscriptions.push(tokio::spawn(async move {
                                            while let Some(message) = subscription.next().await {
                                                let Ok(event) = serde_json::from_slice::<WireEnvelope>(&message.payload) else { continue };
                                                let output = if let Some(private_user) = private_user.as_deref() {
                                                    private_event(event, private_user)
                                                } else {
                                                    Some(public_event(event))
                                                };
                                                if let Some(output) = output {
                                                    if tx.send(output).await.is_err() { break; }
                                                }
                                            }
                                        }));
                                        if sender.send(Message::Text(json!({"type":"subscribed","channel":channel,"ticker":ticker}).to_string().into())).await.is_err() { break; }
                                    }
                                    Err(_) => {
                                        if sender.send(Message::Text(json!({"type":"error","code":"SUBSCRIBE_FAILED"}).to_string().into())).await.is_err() { break; }
                                    }
                                }
                            }
                        } else {
                            let _ = sender.send(Message::Text(json!({"type":"error","code":"UNSUPPORTED_OPERATION"}).to_string().into())).await;
                        }
                    }
                    Message::Ping(payload) => { if sender.send(Message::Pong(payload)).await.is_err() { break; } }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            Some(event) = event_rx.recv() => {
                if sender.send(Message::Text(event.to_string().into())).await.is_err() { break; }
            }
            else => break,
        }
    }
    for subscription in subscriptions {
        subscription.abort();
    }
}

fn public_event(event: WireEnvelope) -> Value {
    json!({
        "type": "market_trade",
        "event_id": event.event_id,
        "ticker": event.payload.ticker,
        "global_seq": event.global_seq,
        "contract_seq": event.contract_seq,
        "price_ticks": event.payload.price_ticks,
        "count": event.payload.count,
        "aggressor_side": event.payload.aggressor_side,
    })
}

fn private_event(event: WireEnvelope, user_id: &str) -> Option<Value> {
    let is_maker = event.payload.maker_user_id == user_id;
    let is_taker = event.payload.taker_user_id == user_id;
    if !is_maker && !is_taker {
        return None;
    }
    Some(json!({
        "type": "private_fill",
        "event_id": event.event_id,
        "ticker": event.payload.ticker,
        "global_seq": event.global_seq,
        "contract_seq": event.contract_seq,
        "order_id": if is_maker { event.payload.maker_order_id } else { event.payload.taker_order_id },
        "role": if is_maker { "maker" } else { "taker" },
        "price_ticks": event.payload.price_ticks,
        "count": event.payload.count,
        "aggressor_side": event.payload.aggressor_side,
    }))
}

async fn authenticated_user(
    headers: &HeaderMap,
    auth: &Authenticator,
    pool: &sqlx::PgPool,
) -> Option<String> {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    if let Some(value) = authorization {
        return auth.verify_authorization(value).ok();
    }
    let raw = headers.get("x-api-key")?.to_str().ok()?.trim();
    let hash = hash_api_key(raw);
    let row = sqlx::query("SELECT u.user_id, k.scopes FROM auth.api_keys k JOIN auth.users u ON u.subject_id=k.subject_id WHERE k.key_hash=$1 AND k.revoked_at IS NULL AND u.status='ACTIVE' AND (k.expires_at IS NULL OR k.expires_at > now())")
        .bind(&hash)
        .fetch_optional(pool)
        .await
        .ok()??;
    let user_id: String = row.try_get("user_id").ok()?;
    let scopes: Vec<String> = row.try_get("scopes").ok()?;
    if !scopes
        .iter()
        .any(|scope| scope == "*" || scope == "websocket:read")
    {
        return None;
    }
    let _ = sqlx::query("UPDATE auth.api_keys SET last_used_at=now() WHERE key_hash=$1")
        .bind(hash)
        .execute(pool)
        .await;
    Some(user_id)
}

async fn run_market_subscription(
    nats: async_nats::Client,
    me_addr: String,
    ticker: String,
    tx: mpsc::Sender<Value>,
) {
    let book_subject = format!("md.book.{ticker}");
    let trade_subject = format!("md.trade.{ticker}");
    let Ok(mut book_subscription) = nats.subscribe(book_subject).await else {
        return;
    };
    let Ok(mut trade_subscription) = nats.subscribe(trade_subject).await else {
        return;
    };
    let Ok(me) = MeCoreClient::connect_lazy(me_addr, Duration::from_secs(3)) else {
        return;
    };
    let snapshot = me.get_book_snapshot(ticker.clone(), 25);
    tokio::pin!(snapshot);
    let mut buffered = Vec::new();
    let snapshot = loop {
        tokio::select! {
            message = book_subscription.next() => {
                let Some(message) = message else { return };
                if let Ok(event) = serde_json::from_slice::<WireBookEnvelope>(&message.payload) {
                    buffered.push(MarketEvent::Book(event));
                }
            }
            message = trade_subscription.next() => {
                let Some(message) = message else { return };
                if let Ok(event) = serde_json::from_slice::<WireEnvelope>(&message.payload) {
                    buffered.push(MarketEvent::Trade(event));
                }
            }
            result = &mut snapshot => break result,
        }
    };
    let Ok(snapshot) = snapshot else { return };
    if tx.send(json!({
        "type": "market_book_snapshot",
        "ticker": snapshot.ticker,
        "seq": snapshot.seq,
        "book_seq": snapshot.book_seq,
        "bids": snapshot.bids.iter().map(|level| json!({"price_ticks":level.price_ticks,"total_qty":level.total_qty,"order_count":level.order_count})).collect::<Vec<_>>(),
        "asks": snapshot.asks.iter().map(|level| json!({"price_ticks":level.price_ticks,"total_qty":level.total_qty,"order_count":level.order_count})).collect::<Vec<_>>(),
    })).await.is_err() { return; }
    buffered.sort_by_key(market_event_sequence);
    for event in buffered {
        match event {
            MarketEvent::Book(event) => {
                if event.contract_seq.unwrap_or(0) > snapshot.seq
                    && tx.send(book_delta_event(event)).await.is_err()
                {
                    return;
                }
            }
            MarketEvent::Trade(event) => {
                if tx.send(public_event(event)).await.is_err() {
                    return;
                }
            }
        }
    }
    loop {
        tokio::select! {
            message = book_subscription.next() => {
                let Some(message) = message else { return };
                let Ok(event) = serde_json::from_slice::<WireBookEnvelope>(&message.payload) else {
                    continue;
                };
                if tx.send(book_delta_event(event)).await.is_err() {
                    return;
                }
            }
            message = trade_subscription.next() => {
                let Some(message) = message else { return };
                let Ok(event) = serde_json::from_slice::<WireEnvelope>(&message.payload) else {
                    continue;
                };
                if tx.send(public_event(event)).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn market_event_sequence(event: &MarketEvent) -> (u64, u8) {
    match event {
        MarketEvent::Book(event) => (event.global_seq, 1),
        MarketEvent::Trade(event) => (event.global_seq, 0),
    }
}

fn book_delta_event(event: WireBookEnvelope) -> Value {
    json!({
        "type": "market_book_delta",
        "event_id": event.event_id,
        "ticker": event.payload.ticker,
        "global_seq": event.global_seq,
        "contract_seq": event.contract_seq,
        "book_side": event.payload.book_side.and_then(book_side_name),
        "book_seq": event.payload.book_seq.or(event.contract_seq),
        "side": event.payload.side,
        "price_ticks": event.payload.price_ticks,
        "qty_delta": event.payload.qty_delta,
        "new_total_qty": event.payload.new_total_qty,
        "new_order_count": event.payload.new_order_count,
    })
}

fn book_side_name(value: i32) -> Option<&'static str> {
    match BookSide::try_from(value).ok()? {
        BookSide::Bid => Some("BID"),
        BookSide::Ask => Some("ASK"),
        BookSide::Unspecified => None,
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill() -> WireEnvelope {
        WireEnvelope {
            event_id: "fill-1".to_owned(),
            global_seq: 7,
            contract_seq: Some(3),
            payload: WireFill {
                ticker: "T".to_owned(),
                maker_user_id: "maker".to_owned(),
                taker_user_id: "taker".to_owned(),
                maker_order_id: "maker-order".to_owned(),
                taker_order_id: "taker-order".to_owned(),
                price_ticks: 42,
                count: 5,
                aggressor_side: 1,
            },
        }
    }

    #[test]
    fn public_events_do_not_expose_users_or_orders() {
        let value = public_event(fill());
        assert!(value.get("maker_user_id").is_none());
        assert!(value.get("maker_order_id").is_none());
        assert_eq!(value["price_ticks"], 42);
    }

    #[test]
    fn market_events_are_ordered_by_global_sequence() {
        let trade = MarketEvent::Trade(fill());
        let WireFill { ticker, .. } = fill().payload;
        let book = WireBookEnvelope {
            event_id: "book-1".to_owned(),
            global_seq: 8,
            contract_seq: Some(4),
            payload: WireBookDelta {
                ticker,
                side: 1,
                book_side: Some(1),
                book_seq: Some(4),
                price_ticks: 42,
                qty_delta: 1,
                new_total_qty: 1,
                new_order_count: Some(1),
            },
        };
        assert!(market_event_sequence(&trade) < market_event_sequence(&MarketEvent::Book(book)));
    }

    #[test]
    fn private_events_are_filtered_and_redacted() {
        let event = fill();
        assert!(private_event(event.clone(), "unknown").is_none());
        let value = private_event(event, "taker").expect("taker receives own fill");
        assert_eq!(value["order_id"], "taker-order");
        assert!(value.get("maker_user_id").is_none());
    }
}
