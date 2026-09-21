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
use sarvex_auth::Authenticator;
use sarvex_me_client::MeCoreClient;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle};

#[derive(Clone)]
struct AppState {
    nats_url: String,
    me_addr: String,
    auth: Arc<Authenticator>,
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
    price_ticks: i64,
    qty_delta: i64,
    new_total_qty: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    sarvex_runtime::init_tracing("gw-ws");
    let state = AppState {
        nats_url: env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        me_addr: env::var("ME_CORE_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50054".to_owned()),
        auth: Arc::new(Authenticator::from_env()?),
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
    let user_id = authenticated_user(&headers, &state.auth);
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

fn authenticated_user(headers: &HeaderMap, auth: &Authenticator) -> Option<String> {
    let raw = headers.get("authorization")?.to_str().ok()?;
    auth.verify_authorization(raw).ok()
}

async fn run_market_subscription(
    nats: async_nats::Client,
    me_addr: String,
    ticker: String,
    tx: mpsc::Sender<Value>,
) {
    let subject = format!("md.book.{ticker}");
    let Ok(mut subscription) = nats.subscribe(subject).await else {
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
            message = subscription.next() => {
                let Some(message) = message else { return };
                if let Ok(event) = serde_json::from_slice::<WireBookEnvelope>(&message.payload) {
                    buffered.push(event);
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
        "bids": snapshot.bids.iter().map(|level| json!({"price_ticks":level.price_ticks,"total_qty":level.total_qty,"order_count":level.order_count})).collect::<Vec<_>>(),
        "asks": snapshot.asks.iter().map(|level| json!({"price_ticks":level.price_ticks,"total_qty":level.total_qty,"order_count":level.order_count})).collect::<Vec<_>>(),
    })).await.is_err() { return; }
    buffered.sort_by_key(|event: &WireBookEnvelope| {
        (event.contract_seq.unwrap_or(0), event.global_seq)
    });
    for event in buffered {
        if event.contract_seq.unwrap_or(0) > snapshot.seq
            && tx.send(book_delta_event(event)).await.is_err()
        {
            return;
        }
    }
    while let Some(message) = subscription.next().await {
        let Ok(event) = serde_json::from_slice::<WireBookEnvelope>(&message.payload) else {
            continue;
        };
        if tx.send(book_delta_event(event)).await.is_err() {
            return;
        }
    }
}

fn book_delta_event(event: WireBookEnvelope) -> Value {
    json!({
        "type": "market_book_delta",
        "event_id": event.event_id,
        "ticker": event.payload.ticker,
        "global_seq": event.global_seq,
        "contract_seq": event.contract_seq,
        "side": event.payload.side,
        "price_ticks": event.payload.price_ticks,
        "qty_delta": event.payload.qty_delta,
        "new_total_qty": event.payload.new_total_qty,
    })
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
    fn private_events_are_filtered_and_redacted() {
        let event = fill();
        assert!(private_event(event.clone(), "unknown").is_none());
        let value = private_event(event, "taker").expect("taker receives own fill");
        assert_eq!(value["order_id"], "taker-order");
        assert!(value.get("maker_user_id").is_none());
    }
}
