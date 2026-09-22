#![allow(clippy::result_large_err)]

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use prost_types::Timestamp;
use sarvex_auth::{AuthMode, Authenticator};
use sarvex_contracts::sarvex::v1::{
    get_order_request, ledger_client::LedgerClient, order_router_client::OrderRouterClient,
    position_client::PositionClient, ref_data_client::RefDataClient, Action, Balance, BookSnapshot,
    CancelOrderRequest, Contract, ContractState, Fill, GetAccountHistoryRequest, GetBalanceRequest,
    GetContractRequest, GetOpenInterestRequest, GetOrderRequest, GetPositionRequest,
    ListContractsRequest, ListFillsRequest, ListOrdersRequest, ListPositionsRequest, Order,
    OrderStatus, SelfTradePreventionType, Side, SubmitOrderRequest, TimeInForce,
};
use sarvex_db::connect;
use sarvex_me_client::{MeCoreClient, MeCoreError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::{
    env,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::TcpStream, task::JoinSet, time::timeout};
use tonic::{
    transport::{Channel, Endpoint},
    Code,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[derive(Clone)]
struct AppState {
    auth: Arc<Authenticator>,
    pool: PgPool,
    refdata: RefDataClient<Channel>,
    orders: OrderRouterClient<Channel>,
    ledger: LedgerClient<Channel>,
    positions: PositionClient<Channel>,
    me: MeCoreClient,
}

#[derive(Debug, Deserialize)]
struct MarketQuery {
    state: Option<String>,
    series_ticker: Option<String>,
    limit: Option<i32>,
    cursor: Option<String>,
}
#[derive(Debug, Deserialize)]
struct LoginRequest {
    user_id: String,
    password: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
struct OrderInput {
    client_order_id: String,
    ticker: String,
    side: String,
    action: String,
    #[serde(default)]
    order_type: Option<String>,
    #[serde(rename = "type", default)]
    type_alias: Option<String>,
    #[serde(default)]
    price_ticks: i64,
    #[serde(default)]
    count: i64,
    #[serde(default)]
    tif: String,
    #[serde(default)]
    post_only: bool,
    #[serde(default)]
    reduce_only: bool,
    #[serde(default)]
    stp: String,
    expires_at: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
struct DepositInput {
    amount_micro_usdc: Option<i64>,
    amount_usdc: Option<i64>,
    note: Option<String>,
}
#[derive(Debug, Deserialize)]
struct HistoryQuery {
    limit: Option<i32>,
    cursor: Option<String>,
}
#[derive(Debug, Deserialize)]
struct OrderQuery {
    ticker: Option<String>,
    status: Option<String>,
    limit: Option<i32>,
    cursor: Option<String>,
}
#[derive(Debug, Deserialize)]
struct FillQuery {
    from_global_seq: Option<u64>,
    to_global_seq: Option<u64>,
    limit: Option<i32>,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OrderBookQuery {
    depth: Option<i32>,
}

#[derive(Debug, Serialize)]
struct HealthItem {
    name: String,
    kind: String,
    status: String,
    message: String,
    target: String,
    latency_ms: u128,
    checked_at: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    sarvex_runtime::init_tracing("gw-rest");
    let auth = Arc::new(Authenticator::from_env()?);
    let pool = connect().await?;
    let state = AppState {
        auth,
        pool,
        refdata: RefDataClient::new(grpc_channel("REFDATA_ADDR", "http://127.0.0.1:50051")?),
        orders: OrderRouterClient::new(grpc_channel(
            "ORDER_ROUTER_ADDR",
            "http://127.0.0.1:50055",
        )?),
        ledger: LedgerClient::new(grpc_channel("LEDGER_ADDR", "http://127.0.0.1:50052")?),
        positions: PositionClient::new(grpc_channel("POSITION_ADDR", "http://127.0.0.1:50056")?),
        me: MeCoreClient::connect_lazy(
            env::var("ME_CORE_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50054".to_owned()),
            Duration::from_secs(2),
        )?,
    };
    let port = env::var("HTTP_PORT").unwrap_or_else(|_| "18080".to_owned());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .route("/v1/health/overview", get(health_overview))
        .route("/v1/auth/login", post(login))
        .route("/v1/markets", get(list_markets))
        .route("/v1/markets/{ticker}", get(get_market))
        .route("/v1/markets/{ticker}/orderbook", get(get_orderbook))
        .route("/v1/markets/{ticker}/fills", get(list_market_fills))
        .route("/v1/orders", get(list_orders).post(submit_order))
        .route("/v1/orders/{order_id}", get(get_order))
        .route("/v1/orders/{order_id}/cancel", post(cancel_order))
        .route("/v1/account/balance", get(get_balance))
        .route("/v1/account/history", get(get_history))
        .route("/v1/positions", get(list_positions))
        .route("/v1/positions/{ticker}", get(get_position))
        .route("/v1/markets/{ticker}/open-interest", get(get_open_interest))
        .route("/v1/demo/deposits/credit", post(demo_deposit))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn grpc_channel(name: &str, default: &str) -> anyhow::Result<Channel> {
    Ok(
        Endpoint::from_shared(env::var(name).unwrap_or_else(|_| default.to_owned()))?
            .connect_lazy(),
    )
}
async fn healthz() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({"status":"ok","service":"gw-rest"})),
    )
}
async fn readyz() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({"status":"ready","service":"gw-rest"})),
    )
}

async fn health_overview() -> impl IntoResponse {
    let checked_at = Utc::now().to_rfc3339();
    let mut items = vec![HealthItem {
        name: "gw-rest".to_owned(),
        kind: "backend".to_owned(),
        status: "running".to_owned(),
        message: "ready".to_owned(),
        target: "self".to_owned(),
        latency_ms: 0,
        checked_at: checked_at.clone(),
    }];

    let targets = [
        ("postgres", "infrastructure", "postgres:5432"),
        ("nats", "infrastructure", "nats:4222"),
        ("refdata-svc", "backend", "refdata-svc:8080"),
        ("ledger-svc", "backend", "ledger-svc:8081"),
        ("risk-svc", "backend", "risk-svc:8082"),
        ("me-core", "backend", "me-core:50054"),
        ("order-router", "backend", "order-router:8085"),
        ("position-svc", "backend", "position-svc:8086"),
        ("marketdata-svc", "backend", "marketdata-svc:8087"),
        ("oracle-svc", "backend", "oracle-svc:8088"),
        ("settlement-svc", "backend", "settlement-svc:8089"),
        ("gw-ws", "backend", "gw-ws:8082"),
        ("trade-bots", "backend", "trade-bots:8090"),
    ];
    let mut checks = JoinSet::new();
    for (name, kind, target) in targets {
        checks.spawn(probe_health(
            name.to_owned(),
            kind.to_owned(),
            target.to_owned(),
            checked_at.clone(),
        ));
    }
    while let Some(result) = checks.join_next().await {
        if let Ok(item) = result {
            items.push(item);
        }
    }
    items.sort_by(|left, right| left.name.cmp(&right.name));
    let running = items.iter().filter(|item| item.status == "running").count();
    Json(json!({
        "generated_at": Utc::now().to_rfc3339(),
        "summary": {
            "running": running,
            "total": items.len(),
            "not_running": items.len() - running,
        },
        "items": items,
    }))
}

async fn probe_health(
    name: String,
    kind: String,
    target: String,
    checked_at: String,
) -> HealthItem {
    let started = Instant::now();
    let (status, message) =
        match timeout(Duration::from_millis(700), TcpStream::connect(&target)).await {
            Ok(Ok(_)) => ("running".to_owned(), "tcp reachable".to_owned()),
            Ok(Err(error)) => ("not_running".to_owned(), error.to_string()),
            Err(_) => ("not_running".to_owned(), "connection timed out".to_owned()),
        };
    HealthItem {
        name,
        kind,
        status,
        message,
        target,
        latency_ms: started.elapsed().as_millis(),
        checked_at,
    }
}
async fn metrics() -> Response {
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], "# HELP sarvex_gateway_up Gateway health state.\n# TYPE sarvex_gateway_up gauge\nsarvex_gateway_up{service=\"gw-rest\"} 1\n").into_response()
}

async fn login(State(state): State<AppState>, Json(input): Json<LoginRequest>) -> Response {
    let user_id = input.user_id.trim();
    if user_id.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "user_id is required",
        );
    }
    if state.auth.mode() == AuthMode::Jwt {
        let expected = env::var("AUTH_LOGIN_SECRET").unwrap_or_default();
        if expected.is_empty() || input.password.as_deref() != Some(expected.as_str()) {
            return error_response(
                StatusCode::UNAUTHORIZED,
                "UNAUTHENTICATED",
                "invalid credentials",
            );
        }
    }
    match state.auth.issue(user_id, "trader") {
        Ok(token) => {
            Json(json!({"token":token,"token_type":"Bearer","user_id":user_id})).into_response()
        }
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "AUTH_ERROR",
            &error.to_string(),
        ),
    }
}

async fn list_markets(State(state): State<AppState>, Query(query): Query<MarketQuery>) -> Response {
    let mut client = state.refdata;
    match rpc(client.list_contracts(ListContractsRequest {
        state: query.state.as_deref().and_then(state_value).unwrap_or(0),
        series_ticker: query.series_ticker.unwrap_or_default(), limit: query.limit.unwrap_or(50).clamp(1,500),
        cursor: query.cursor.unwrap_or_default(),
    })).await {
        Ok(response) => Json(json!({"contracts":response.contracts.iter().map(contract_json).collect::<Vec<_>>(),"next_cursor":response.next_cursor})).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn get_market(State(state): State<AppState>, Path(ticker): Path<String>) -> Response {
    let mut client = state.refdata;
    match rpc(client.get_contract(GetContractRequest { ticker })).await {
        Ok(contract) => Json(contract_json(&contract)).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn get_orderbook(
    State(state): State<AppState>,
    Path(ticker): Path<String>,
    Query(query): Query<OrderBookQuery>,
) -> Response {
    let depth = query.depth.unwrap_or(20).clamp(1, 100);
    match state.me.get_book_snapshot(ticker, depth).await {
        Ok(snapshot) => Json(book_snapshot_json(&snapshot)).into_response(),
        Err(error) => me_core_error(error),
    }
}

async fn list_market_fills(
    State(state): State<AppState>,
    Path(ticker): Path<String>,
    Query(query): Query<FillQuery>,
) -> Response {
    let mut client = state.orders;
    match rpc(client.list_fills(ListFillsRequest { ticker, from_global_seq: query.from_global_seq.unwrap_or(0), to_global_seq: query.to_global_seq.unwrap_or(0), limit: query.limit.unwrap_or(100).clamp(1,500), cursor: query.cursor.unwrap_or_default() })).await {
        Ok(response) => Json(json!({"fills":response.fills.iter().map(fill_record_json).collect::<Vec<_>>(),"next_cursor":response.next_cursor})).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn submit_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<OrderInput>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let idempotency_key = match required_header(&headers, "idempotency-key") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let request_body = json!(&input);
    if let Some(response) = match replay_idempotency(
        &state.pool,
        &user_id,
        "POST",
        "/v1/orders",
        &idempotency_key,
        &request_body,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return response,
    } {
        return response;
    }
    let side = match side_value(&input.side) {
        Some(value) => value,
        None => return error_response(StatusCode::BAD_REQUEST, "INVALID_ARGUMENT", "invalid side"),
    };
    let action = match action_value(&input.action) {
        Some(value) => value,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENT",
                "invalid action",
            )
        }
    };
    let market = input
        .order_type
        .as_deref()
        .or(input.type_alias.as_deref())
        .is_some_and(|value| value.eq_ignore_ascii_case("market"));
    let price_ticks = if market {
        let contract = match state
            .refdata
            .clone()
            .get_contract(GetContractRequest {
                ticker: input.ticker.clone(),
            })
            .await
        {
            Ok(contract) => contract.into_inner(),
            Err(error) => return grpc_error(error),
        };
        market_protection_price(side, action, &contract)
    } else {
        input.price_ticks
    };
    let expires_at = match input.expires_at.as_deref() {
        Some(value) => match parse_timestamp(value) {
            Ok(value) => value,
            Err(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "INVALID_ARGUMENT",
                    "invalid expires_at",
                )
            }
        },
        None => None,
    };
    let mut client = state.orders;
    let value = match rpc(client.submit_order(SubmitOrderRequest {
        user_id: user_id.clone(),
        client_order_id: input.client_order_id,
        ticker: input.ticker,
        side,
        action,
        price_ticks,
        count: input.count,
        tif: if market {
            TimeInForce::Ioc as i32
        } else {
            tif_value(&input.tif)
        },
        post_only: input.post_only && !market,
        reduce_only: input.reduce_only,
        stp: stp_value(&input.stp),
        expires_at,
        idempotency_key: idempotency_key.clone(),
    }))
    .await
    {
        Ok(response) => {
            json!({"order":response.order.as_ref().map(order_json),"fills":response.fills.iter().map(fill_json).collect::<Vec<_>>(),"reject_code":response.reject_code,"reject_reason":response.reject_reason})
        }
        Err(error) => return grpc_error(error),
    };
    if let Err(response) = store_idempotency(
        &state.pool,
        &user_id,
        "POST",
        "/v1/orders",
        &idempotency_key,
        &request_body,
        &value,
        StatusCode::OK,
    )
    .await
    {
        return response;
    }
    Json(value).into_response()
}

async fn list_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<OrderQuery>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let status = match query.status.as_deref() {
        Some(value) => match order_status_value(value) {
            Some(value) => value,
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "INVALID_ARGUMENT",
                    "invalid order status",
                )
            }
        },
        None => 0,
    };
    let mut client = state.orders;
    match rpc(client.list_orders(ListOrdersRequest { user_id, ticker: query.ticker.unwrap_or_default(), status, limit: query.limit.unwrap_or(100).clamp(1,500), cursor: query.cursor.unwrap_or_default() })).await {
        Ok(response) => Json(json!({"orders":response.orders.iter().map(order_json).collect::<Vec<_>>(),"next_cursor":response.next_cursor})).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn get_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.orders;
    match rpc(client.get_order(GetOrderRequest {
        user_id,
        key: Some(get_order_request::Key::OrderId(order_id)),
    }))
    .await
    {
        Ok(order) => Json(order_json(&order)).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn cancel_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let idempotency_key = match required_header(&headers, "idempotency-key") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let request_path = format!("/v1/orders/{order_id}/cancel");
    let request_body = json!({"order_id": order_id});
    if let Some(response) = match replay_idempotency(
        &state.pool,
        &user_id,
        "POST",
        &request_path,
        &idempotency_key,
        &request_body,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return response,
    } {
        return response;
    }
    let mut client = state.orders;
    let value = match rpc(client.cancel_order(CancelOrderRequest {
        user_id: user_id.clone(),
        order_id,
        client_order_id: String::new(),
    }))
    .await
    {
        Ok(response) => {
            json!({"order":response.order.as_ref().map(order_json),"reject_code":response.reject_code,"reject_reason":response.reject_reason})
        }
        Err(error) => return grpc_error(error),
    };
    if let Err(response) = store_idempotency(
        &state.pool,
        &user_id,
        "POST",
        &request_path,
        &idempotency_key,
        &request_body,
        &value,
        StatusCode::OK,
    )
    .await
    {
        return response;
    }
    Json(value).into_response()
}

async fn get_balance(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.ledger;
    match rpc(client.get_balance(GetBalanceRequest { user_id })).await {
        Ok(balance) => Json(balance_json(&balance)).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn get_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.ledger;
    match rpc(client.get_account_history(GetAccountHistoryRequest {
        user_id,
        limit: query.limit.unwrap_or(100).clamp(1, 500),
        cursor: query.cursor.unwrap_or_default(),
    }))
    .await
    {
        Ok(response) => {
            Json(json!({"entries":response.entries.iter().map(history_entry_json).collect::<Vec<_>>(),"next_cursor":response.next_cursor}))
                .into_response()
        }
        Err(error) => grpc_error(error),
    }
}

async fn list_positions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.positions;
    match rpc(client.list_positions(ListPositionsRequest { user_id, include_closed: true })).await {
        Ok(response) => Json(json!({"positions":response.positions.iter().map(position_json).collect::<Vec<_>>(),"next_cursor":response.next_cursor})).into_response(), Err(error) => grpc_error(error),
    }
}

async fn get_position(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(ticker): Path<String>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.positions;
    match rpc(client.get_position(GetPositionRequest { user_id, ticker })).await {
        Ok(position) => Json(position_json(&position)).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn get_open_interest(State(state): State<AppState>, Path(ticker): Path<String>) -> Response {
    let mut client = state.positions;
    match rpc(client.get_open_interest(GetOpenInterestRequest { ticker })).await {
        Ok(value) => Json(json!({"ticker":value.ticker,"total_open_long":value.total_open_long,"total_open_short":value.total_open_short})).into_response(), Err(error) => grpc_error(error),
    }
}

async fn demo_deposit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<DepositInput>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let idempotency_key = match required_header(&headers, "idempotency-key") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let request_body = json!(&input);
    if let Some(response) = match replay_idempotency(
        &state.pool,
        &user_id,
        "POST",
        "/v1/demo/deposits/credit",
        &idempotency_key,
        &request_body,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return response,
    } {
        return response;
    }
    let amount = input
        .amount_micro_usdc
        .or_else(|| {
            input
                .amount_usdc
                .map(|value| value.saturating_mul(1_000_000))
        })
        .unwrap_or(0);
    if amount <= 0 || amount > 1_000_000_000_000 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "valid deposit amount is required",
        );
    }
    let mut client = state.ledger;
    match rpc(client.admin_credit_deposit(
        sarvex_contracts::sarvex::v1::AdminCreditDepositRequest {
            user_id: user_id.clone(),
            amount_micro_usdc: amount,
            note: input
                .note
                .unwrap_or_else(|| "frontend demo deposit".to_owned()),
        },
    ))
    .await
    {
        Ok(_) => match rpc(client.get_balance(GetBalanceRequest {
            user_id: user_id.clone(),
        }))
        .await
        {
            Ok(balance) => {
                let value = json!({"ok":true,"balance":balance_json(&balance)});
                if let Err(response) = store_idempotency(
                    &state.pool,
                    &user_id,
                    "POST",
                    "/v1/demo/deposits/credit",
                    &idempotency_key,
                    &request_body,
                    &value,
                    StatusCode::OK,
                )
                .await
                {
                    return response;
                }
                Json(value).into_response()
            }
            Err(error) => grpc_error(error),
        },
        Err(error) => grpc_error(error),
    }
}

async fn rpc<T>(
    future: impl std::future::Future<Output = Result<tonic::Response<T>, tonic::Status>>,
) -> Result<T, tonic::Status> {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .map_err(|_| tonic::Status::deadline_exceeded("gateway upstream timeout"))?
        .map(|response| response.into_inner())
}

fn authenticated_user(headers: &HeaderMap, auth: &Authenticator) -> Result<String, Response> {
    let raw = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    auth.verify_authorization(raw).map_err(|error| {
        error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHENTICATED",
            &error.to_string(),
        )
    })
}

fn required_header(headers: &HeaderMap, name: &str) -> Result<String, Response> {
    let value = headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim();
    if value.is_empty() {
        Err(error_response(
            StatusCode::BAD_REQUEST,
            "IDEMPOTENCY_KEY_REQUIRED",
            "Idempotency-Key header is required",
        ))
    } else {
        Ok(value.to_owned())
    }
}

fn request_hash(value: &Value) -> String {
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(value).unwrap_or_default());
    format!("{:x}", digest.finalize())
}

async fn replay_idempotency(
    pool: &PgPool,
    user_id: &str,
    method: &str,
    path: &str,
    key: &str,
    request: &Value,
) -> Result<Option<Response>, Response> {
    let hash = request_hash(request);
    sqlx::query("DELETE FROM gateway.idempotency_records WHERE user_id=$1 AND method=$2 AND request_path=$3 AND idempotency_key=$4 AND expires_at <= now()")
        .bind(user_id)
        .bind(method)
        .bind(path)
        .bind(key)
        .execute(pool)
        .await
        .map_err(|error| error_response(StatusCode::SERVICE_UNAVAILABLE, "IDEMPOTENCY_STORE_UNAVAILABLE", &error.to_string()))?;
    let row = sqlx::query("SELECT request_hash, response_status, response_body FROM gateway.idempotency_records WHERE user_id=$1 AND method=$2 AND request_path=$3 AND idempotency_key=$4 AND expires_at > now()")
        .bind(user_id).bind(method).bind(path).bind(key).fetch_optional(pool).await
        .map_err(|error| error_response(StatusCode::SERVICE_UNAVAILABLE, "IDEMPOTENCY_STORE_UNAVAILABLE", &error.to_string()))?;
    let Some(row) = row else { return Ok(None) };
    let stored_hash: String = row.try_get("request_hash").map_err(|error| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "IDEMPOTENCY_STORE_ERROR",
            &error.to_string(),
        )
    })?;
    if stored_hash != hash {
        return Err(error_response(
            StatusCode::CONFLICT,
            "IDEMPOTENCY_KEY_REUSED",
            "idempotency key was used with a different request",
        ));
    }
    let status: i32 = row.try_get("response_status").map_err(|error| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "IDEMPOTENCY_STORE_ERROR",
            &error.to_string(),
        )
    })?;
    let body: Value = row.try_get("response_body").map_err(|error| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "IDEMPOTENCY_STORE_ERROR",
            &error.to_string(),
        )
    })?;
    let status = StatusCode::from_u16(status as u16).unwrap_or(StatusCode::OK);
    Ok(Some((status, Json(body)).into_response()))
}

#[allow(clippy::too_many_arguments)]
async fn store_idempotency(
    pool: &PgPool,
    user_id: &str,
    method: &str,
    path: &str,
    key: &str,
    request: &Value,
    response: &Value,
    status: StatusCode,
) -> Result<(), Response> {
    sqlx::query("INSERT INTO gateway.idempotency_records (user_id, method, request_path, idempotency_key, request_hash, response_status, response_body) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (user_id, method, request_path, idempotency_key) DO NOTHING")
        .bind(user_id).bind(method).bind(path).bind(key).bind(request_hash(request))
        .bind(status.as_u16() as i32).bind(response)
        .execute(pool).await
        .map(|_| ())
        .map_err(|error| error_response(StatusCode::SERVICE_UNAVAILABLE, "IDEMPOTENCY_STORE_UNAVAILABLE", &error.to_string()))
}

fn side_value(value: &str) -> Option<i32> {
    match value.trim().to_ascii_uppercase().as_str() {
        "YES" => Some(Side::Yes as i32),
        "NO" => Some(Side::No as i32),
        "LONG" => Some(Side::Long as i32),
        "SHORT" => Some(Side::Short as i32),
        _ => None,
    }
}
fn action_value(value: &str) -> Option<i32> {
    match value.trim().to_ascii_uppercase().as_str() {
        "BUY" => Some(Action::Buy as i32),
        "SELL" => Some(Action::Sell as i32),
        _ => None,
    }
}

fn market_protection_price(side: i32, action: i32, contract: &Contract) -> i64 {
    let side_is_positive = side == Side::Yes as i32 || side == Side::Long as i32;
    let action_is_buy = action == Action::Buy as i32;
    if side_is_positive == action_is_buy {
        contract.max_price_ticks
    } else {
        contract.min_price_ticks
    }
}

fn tif_value(value: &str) -> i32 {
    match value.trim().to_ascii_uppercase().as_str() {
        "IOC" => TimeInForce::Ioc as i32,
        "FOK" => TimeInForce::Fok as i32,
        _ => TimeInForce::Gtc as i32,
    }
}
fn stp_value(value: &str) -> i32 {
    match value.trim().to_ascii_uppercase().as_str() {
        "TAKER_AT_CROSS" => SelfTradePreventionType::TakerAtCross as i32,
        "MAKER" => SelfTradePreventionType::Maker as i32,
        _ => SelfTradePreventionType::Unspecified as i32,
    }
}
fn state_value(value: &str) -> Option<i32> {
    match value.trim().to_ascii_uppercase().as_str() {
        "DRAFT" => Some(ContractState::Draft as i32),
        "LISTED" => Some(ContractState::Listed as i32),
        "OPEN" => Some(ContractState::Open as i32),
        "HALTED" => Some(ContractState::Halted as i32),
        "CLOSED" => Some(ContractState::Closed as i32),
        "RESOLVING" => Some(ContractState::Resolving as i32),
        "SETTLED" => Some(ContractState::Settled as i32),
        "CANCELLED" => Some(ContractState::Cancelled as i32),
        _ => None,
    }
}
fn order_status_value(value: &str) -> Option<i32> {
    Some(match value.trim().to_ascii_uppercase().as_str() {
        "PENDING" => OrderStatus::Pending,
        "OPEN" => OrderStatus::Open,
        "PARTIAL" => OrderStatus::Partial,
        "FILLED" => OrderStatus::Filled,
        "CANCELLED" => OrderStatus::Cancelled,
        "REJECTED" => OrderStatus::Rejected,
        "EXPIRED" => OrderStatus::Expired,
        _ => return None,
    } as i32)
}
fn parse_timestamp(value: &str) -> Result<Option<Timestamp>, ()> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| ())?
        .with_timezone(&Utc);
    Ok(Some(Timestamp {
        seconds: parsed.timestamp(),
        nanos: parsed.timestamp_subsec_nanos() as i32,
    }))
}

fn contract_json(contract: &Contract) -> Value {
    json!({"ticker":contract.ticker,"event_ticker":contract.event_ticker,"series_ticker":contract.series_ticker,"kind":contract.kind,"question":contract.question,"underlying":contract.underlying,"tick_size":contract.tick_size,"min_price_ticks":contract.min_price_ticks,"max_price_ticks":contract.max_price_ticks,"lower_bound_ticks":contract.lower_bound_ticks,"upper_bound_ticks":contract.upper_bound_ticks,"multiplier_micro_usdc":contract.multiplier_micro_usdc,"max_order_size":contract.max_order_size,"position_limit_per_user":contract.position_limit_per_user,"state":contract.state,"listed_at":timestamp_json(contract.listed_at.as_ref()),"open_at":timestamp_json(contract.open_at.as_ref()),"close_at":timestamp_json(contract.close_at.as_ref()),"expected_resolution_at":timestamp_json(contract.expected_resolution_at.as_ref()),"settlement_source":contract.settlement_source,"oracle_policy":contract.oracle_policy,"settlement_rule":contract.settlement_rule.as_ref().map(struct_json),"close_global_seq":contract.close_global_seq})
}
fn order_json(order: &Order) -> Value {
    json!({"order_id":order.order_id,"client_order_id":order.client_order_id,"user_id":order.user_id,"ticker":order.ticker,"side":enum_name(Side::try_from(order.side).ok()),"action":enum_name(Action::try_from(order.action).ok()),"price_ticks":order.price_ticks,"count":order.count,"filled_count":order.filled_count,"remaining_count":order.remaining_count,"tif":enum_name(TimeInForce::try_from(order.tif).ok()),"post_only":order.post_only,"reduce_only":order.reduce_only,"stp":enum_name(SelfTradePreventionType::try_from(order.stp).ok()),"status":enum_name(OrderStatus::try_from(order.status).ok()),"created_at":timestamp_json(order.created_at.as_ref()),"updated_at":timestamp_json(order.updated_at.as_ref()),"expires_at":timestamp_json(order.expires_at.as_ref()),"hold_id":order.hold_id,"avg_fill_price_ticks":order.avg_fill_price_ticks})
}
fn fill_json(fill: &Fill) -> Value {
    json!({"fill_id":fill.fill_id,"order_id":fill.order_id,"ticker":fill.ticker,"price_ticks":fill.price_ticks,"count":fill.count,"aggressor_side":enum_name(Side::try_from(fill.aggressor_side).ok()),"fee_micro_usdc":fill.fee_micro_usdc,"ts":timestamp_json(fill.ts.as_ref()),"seq":fill.seq})
}
fn fill_record_json(fill: &sarvex_contracts::sarvex::v1::FillRecord) -> Value {
    json!({"fill_id":fill.fill_id,"ticker":fill.ticker,"global_seq":fill.global_seq,"contract_seq":fill.contract_seq,"maker_order_id":fill.maker_order_id,"taker_order_id":fill.taker_order_id,"maker_user_id":fill.maker_user_id,"taker_user_id":fill.taker_user_id,"maker_side":enum_name(Side::try_from(fill.maker_side).ok()),"maker_action":enum_name(Action::try_from(fill.maker_action).ok()),"taker_side":enum_name(Side::try_from(fill.taker_side).ok()),"taker_action":enum_name(Action::try_from(fill.taker_action).ok()),"price_ticks":fill.price_ticks,"count":fill.count,"aggressor_side":enum_name(Side::try_from(fill.aggressor_side).ok()),"maker_fee_micro_usdc":fill.maker_fee_micro_usdc,"taker_fee_micro_usdc":fill.taker_fee_micro_usdc,"ts":timestamp_json(fill.ts.as_ref())})
}
fn balance_json(balance: &Balance) -> Value {
    json!({"user_id":balance.user_id,"cash_micro_usdc":balance.cash_micro_usdc,"held_micro_usdc":balance.held_micro_usdc,"total_micro_usdc":balance.total_micro_usdc})
}
fn history_entry_json(entry: &sarvex_contracts::sarvex::v1::LedgerEntryRecord) -> Value {
    json!({"tx_id":entry.tx_id,"account_code":entry.account_code,"direction":entry.direction,"amount_micro_usdc":entry.amount_micro_usdc,"running_balance_micro_usdc":entry.running_balance_micro_usdc,"reason_code":entry.reason_code,"posted_at":timestamp_json(entry.posted_at.as_ref()),"memo":entry.memo})
}
fn position_json(position: &sarvex_contracts::sarvex::v1::UserPosition) -> Value {
    json!({"user_id":position.user_id,"ticker":position.ticker,"net_qty":position.net_qty,"avg_cost_micro_usdc":position.avg_cost_micro_usdc,"realized_pnl_micro_usdc":position.realized_pnl_micro_usdc,"unrealized_pnl_micro_usdc":position.unrealized_pnl_micro_usdc,"updated_at":timestamp_json(position.updated_at.as_ref()),"last_global_seq":position.last_global_seq})
}
fn book_snapshot_json(snapshot: &BookSnapshot) -> Value {
    json!({
        "ticker": snapshot.ticker,
        "seq": snapshot.seq,
        "ts": timestamp_json(snapshot.ts.as_ref()),
        "bids": snapshot.bids.iter().map(book_level_json).collect::<Vec<_>>(),
        "asks": snapshot.asks.iter().map(book_level_json).collect::<Vec<_>>(),
    })
}
fn book_level_json(level: &sarvex_contracts::sarvex::v1::PriceLevel) -> Value {
    json!({
        "price_ticks": level.price_ticks,
        "total_qty": level.total_qty,
        "order_count": level.order_count,
    })
}
fn enum_name<T: std::fmt::Debug>(value: Option<T>) -> String {
    value
        .map(|value| format!("{value:?}").to_ascii_uppercase())
        .unwrap_or_else(|| "UNSPECIFIED".to_owned())
}
fn timestamp_json(value: Option<&Timestamp>) -> Option<String> {
    value
        .and_then(|value| DateTime::<Utc>::from_timestamp(value.seconds, value.nanos as u32))
        .map(|value| value.to_rfc3339())
}
fn struct_json(value: &prost_types::Struct) -> Value {
    Value::Object(
        value
            .fields
            .iter()
            .map(|(key, value)| (key.clone(), prost_value_json(value)))
            .collect(),
    )
}
fn prost_value_json(value: &prost_types::Value) -> Value {
    match value.kind.as_ref() {
        Some(prost_types::value::Kind::NullValue(_)) | None => Value::Null,
        Some(prost_types::value::Kind::NumberValue(value)) => json!(value),
        Some(prost_types::value::Kind::StringValue(value)) => json!(value),
        Some(prost_types::value::Kind::BoolValue(value)) => json!(value),
        Some(prost_types::value::Kind::StructValue(value)) => struct_json(value),
        Some(prost_types::value::Kind::ListValue(value)) => {
            Value::Array(value.values.iter().map(prost_value_json).collect())
        }
    }
}
fn grpc_error(error: tonic::Status) -> Response {
    let status = match error.code() {
        Code::InvalidArgument => StatusCode::BAD_REQUEST,
        Code::Unauthenticated => StatusCode::UNAUTHORIZED,
        Code::PermissionDenied => StatusCode::FORBIDDEN,
        Code::NotFound => StatusCode::NOT_FOUND,
        Code::AlreadyExists => StatusCode::CONFLICT,
        Code::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
        Code::Unimplemented => StatusCode::NOT_IMPLEMENTED,
        _ => StatusCode::BAD_GATEWAY,
    };
    error_response(
        status,
        &format!("{:?}", error.code()).to_ascii_uppercase(),
        error.message(),
    )
}

fn me_core_error(error: MeCoreError) -> Response {
    match error {
        MeCoreError::Status(status) => grpc_error(*status),
        MeCoreError::OutcomeUnknown | MeCoreError::Transport(_) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "ME_CORE_UNAVAILABLE",
            &error.to_string(),
        ),
    }
}
fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"error":{"code":code,"message":message}})),
    )
        .into_response()
}
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn demo_token_round_trips_user_identity() {
        let auth = Authenticator::from_env().expect("demo auth config");
        let token = auth.issue("user_42", "trader").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        assert_eq!(authenticated_user(&headers, &auth).unwrap(), "user_42");
    }

    #[test]
    fn mutation_header_is_required() {
        let headers = HeaderMap::new();
        assert!(required_header(&headers, "idempotency-key").is_err());
    }

    #[test]
    fn order_input_mappings_preserve_contract_enums() {
        assert_eq!(side_value("yes"), Some(Side::Yes as i32));
        assert_eq!(action_value("SELL"), Some(Action::Sell as i32));
        assert_eq!(tif_value("IOC"), TimeInForce::Ioc as i32);
        assert_eq!(stp_value("maker"), SelfTradePreventionType::Maker as i32);
    }
}
