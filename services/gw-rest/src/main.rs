#![allow(clippy::result_large_err)]

use axum::{
    extract::{rejection::JsonRejection, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use prost_types::Timestamp;
use sarvex_auth::{
    generate_api_key, hash_password, verify_password, AuthMethod, AuthMode, Authenticator,
};
use sarvex_contracts::sarvex::v1::{
    get_order_request, ledger_client::LedgerClient, oracle_client::OracleClient,
    order_router_client::OrderRouterClient, position_client::PositionClient,
    ref_data_client::RefDataClient, rfq_service_client::RfqServiceClient, risk_client::RiskClient,
    settlement_client::SettlementClient, AcceptQuoteRequest, Action, Balance, BookSnapshot,
    CancelOrderRequest, CancelQuoteRequest, CancelRfqRequest, Contract, ContractKind,
    ContractState, CreateRfqRequest, Event, Fill, GetAccountHistoryRequest, GetBalanceRequest,
    GetContractRequest, GetOpenInterestRequest, GetOrderRequest, GetPositionRequest,
    GetResolutionRequest, GetRfqRequest, GetSettlementRequest, GetUserLimitsRequest,
    ListContractsRequest, ListEventsRequest, ListFillsRequest, ListOrdersRequest,
    ListPositionsRequest, ListQuotesRequest, ListSeriesRequest, Order, OrderStatus, Resolution,
    Rfq, RfqQuote, RfqQuoteStatus, RfqStatus, SelfTradePreventionType, Series, SettlementResult,
    Side, SubmitOrderRequest, SubmitQuoteRequest, TimeInForce, UserLimits,
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
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    auth: Arc<Authenticator>,
    pool: PgPool,
    refdata: RefDataClient<Channel>,
    orders: OrderRouterClient<Channel>,
    ledger: LedgerClient<Channel>,
    risk: RiskClient<Channel>,
    oracle: OracleClient<Channel>,
    settlement: SettlementClient<Channel>,
    positions: PositionClient<Channel>,
    rfq: RfqServiceClient<Channel>,
    me: MeCoreClient,
}

#[derive(Debug, Deserialize)]
struct MarketQuery {
    state: Option<String>,
    series_ticker: Option<String>,
    kind: Option<String>,
    event_ticker: Option<String>,
    underlying: Option<String>,
    category: Option<String>,
    expected_resolution_from: Option<String>,
    expected_resolution_to: Option<String>,
    limit: Option<i32>,
    cursor: Option<String>,
}
#[derive(Debug, Deserialize)]
struct DiscoveryQuery {
    series_ticker: Option<String>,
    expected_resolution_from: Option<String>,
    expected_resolution_to: Option<String>,
    limit: Option<i32>,
    cursor: Option<String>,
}
#[derive(Debug, Deserialize)]
struct LoginRequest {
    user_id: Option<String>,
    email: Option<String>,
    password: Option<String>,
}
#[derive(Debug, Deserialize)]
struct RegisterRequest {
    user_id: String,
    email: String,
    password: String,
    name: Option<String>,
}
#[derive(Debug, Deserialize)]
struct ApiKeyCreateRequest {
    name: String,
    scopes: Option<Vec<String>>,
    expires_at: Option<String>,
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
struct PositionQuery {
    include_closed: Option<bool>,
    limit: Option<i32>,
    cursor: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
struct RfqInput {
    client_rfq_id: String,
    ticker: String,
    side: String,
    action: String,
    requested_count: i64,
    expires_at: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct RfqQuoteInput {
    quote_id: String,
    bid_price_ticks: i64,
    offer_price_ticks: i64,
    available_count: i64,
    expires_at: String,
}
#[derive(Debug, Deserialize)]
struct FillQuery {
    ticker: Option<String>,
    from_global_seq: Option<u64>,
    to_global_seq: Option<u64>,
    order_id: Option<String>,
    from_time: Option<String>,
    to_time: Option<String>,
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
    let pool = connect().await?;
    let auth = Arc::new(Authenticator::from_env()?);
    load_api_keys(&auth, &pool).await?;
    let state = AppState {
        auth,
        pool,
        refdata: RefDataClient::new(grpc_channel("REFDATA_ADDR", "http://127.0.0.1:50051")?),
        orders: OrderRouterClient::new(grpc_channel(
            "ORDER_ROUTER_ADDR",
            "http://127.0.0.1:50055",
        )?),
        ledger: LedgerClient::new(grpc_channel("LEDGER_ADDR", "http://127.0.0.1:50052")?),
        risk: RiskClient::new(grpc_channel("RISK_ADDR", "http://127.0.0.1:50053")?),
        oracle: OracleClient::new(grpc_channel("ORACLE_ADDR", "http://127.0.0.1:50057")?),
        settlement: SettlementClient::new(grpc_channel(
            "SETTLEMENT_ADDR",
            "http://127.0.0.1:50058",
        )?),
        positions: PositionClient::new(grpc_channel("POSITION_ADDR", "http://127.0.0.1:50056")?),
        rfq: RfqServiceClient::new(grpc_channel("RFQ_ADDR", "http://127.0.0.1:50059")?),
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
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/login", post(login))
        .route("/v1/account/profile", get(get_profile))
        .route(
            "/v1/account/api-keys",
            get(list_api_keys).post(create_api_key),
        )
        .route(
            "/v1/account/api-keys/{key_id}",
            delete(revoke_api_key).post(revoke_api_key),
        )
        .route("/v1/markets", get(list_markets))
        .route("/v1/markets/{ticker}", get(get_market))
        .route("/v1/series", get(list_series))
        .route("/v1/events", get(list_events))
        .route("/v1/events/{event_ticker}", get(get_event))
        .route("/v1/events/{event_ticker}/resolution", get(get_resolution))
        .route("/v1/markets/{ticker}/orderbook", get(get_orderbook))
        .route("/v1/markets/{ticker}/fills", get(list_market_fills))
        .route("/v1/account/fills", get(list_account_fills))
        .route("/v1/orders", get(list_orders).post(submit_order))
        .route("/v1/orders/{order_id}", get(get_order))
        .route("/v1/orders/{order_id}/cancel", post(cancel_order))
        .route("/v1/account/balance", get(get_balance))
        .route("/v1/account/risk", get(get_account_risk))
        .route("/v1/account/history", get(get_history))
        .route("/v1/positions", get(list_positions))
        .route("/v1/positions/{ticker}", get(get_position))
        .route("/v1/markets/{ticker}/open-interest", get(get_open_interest))
        .route("/v1/markets/{ticker}/settlement", get(get_settlement))
        .route("/v1/rfqs", post(create_rfq))
        .route("/v1/rfqs/{rfq_id}", get(get_rfq))
        .route(
            "/v1/rfqs/{rfq_id}/quotes",
            get(list_rfq_quotes).post(submit_rfq_quote),
        )
        .route("/v1/rfqs/{rfq_id}/cancel", post(cancel_rfq))
        .route(
            "/v1/rfqs/{rfq_id}/quotes/{quote_id}/cancel",
            post(cancel_rfq_quote),
        )
        .route(
            "/v1/rfqs/{rfq_id}/quotes/{quote_id}/accept",
            post(accept_rfq_quote),
        )
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
        ("rfq-svc", "backend", "rfq-svc:8091"),
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

async fn register(State(state): State<AppState>, Json(input): Json<RegisterRequest>) -> Response {
    let user_id = input.user_id.trim().to_owned();
    let email = input.email.trim().to_owned();
    let email_normalized = email.to_ascii_lowercase();
    let requested_name = input.name.as_deref().unwrap_or_default().trim();
    if !valid_user_id(&user_id) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "user_id must be 3-32 characters using letters, numbers, '_' or '-'",
        );
    }
    if !valid_email(&email_normalized) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "a valid email is required",
        );
    }
    if input.password.len() < 10 || input.password.len() > 128 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "password must be 10-128 characters",
        );
    }
    if requested_name.len() > 80 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "name must be at most 80 characters",
        );
    }
    let display_name = if requested_name.is_empty() {
        user_id.clone()
    } else {
        requested_name.to_owned()
    };
    let password_hash = match hash_password(&input.password) {
        Ok(value) => value,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "AUTH_ERROR",
                &error.to_string(),
            )
        }
    };
    let mut tx = match state.pool.begin().await {
        Ok(value) => value,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DATABASE_ERROR",
                &error.to_string(),
            )
        }
    };
    let subject_id = match sqlx::query_scalar::<_, uuid::Uuid>("INSERT INTO auth.users (user_id, email, email_normalized, display_name) VALUES ($1,$2,$3,$4) RETURNING subject_id")
        .bind(&user_id).bind(&email).bind(&email_normalized).bind(&display_name).fetch_one(&mut *tx).await {
        Ok(value) => value,
        Err(error) if is_unique_violation(&error) => return error_response(StatusCode::CONFLICT, "ALREADY_EXISTS", "user_id or email is already registered"),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "DATABASE_ERROR", &error.to_string()),
    };
    if let Err(error) =
        sqlx::query("INSERT INTO auth.credentials (subject_id, password_hash) VALUES ($1,$2)")
            .bind(subject_id)
            .bind(&password_hash)
            .execute(&mut *tx)
            .await
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DATABASE_ERROR",
            &error.to_string(),
        );
    }
    if let Err(error) = sqlx::query(
        "INSERT INTO risk.user_limits (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(&user_id)
    .execute(&mut *tx)
    .await
    {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DATABASE_ERROR",
            &error.to_string(),
        );
    }
    for code in [
        format!("LIAB:USER:{user_id}:CASH"),
        format!("LIAB:USER:{user_id}:HOLDS"),
    ] {
        if let Err(error) = sqlx::query("INSERT INTO ledger.accounts (account_code, account_type, currency, user_id) VALUES ($1,'LIABILITY','USDC',$2) ON CONFLICT (account_code) DO NOTHING")
            .bind(code).bind(&user_id).execute(&mut *tx).await {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "DATABASE_ERROR", &error.to_string());
        }
    }
    if let Err(error) = sqlx::query("INSERT INTO auth.audit_events (subject_id, user_id, event_type) VALUES ($1,$2,'USER_REGISTERED')")
        .bind(subject_id).bind(&user_id).execute(&mut *tx).await {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "DATABASE_ERROR", &error.to_string());
    }
    if let Err(error) = tx.commit().await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DATABASE_ERROR",
            &error.to_string(),
        );
    }
    let scopes = default_jwt_scopes();
    match state.auth.issue_with_scopes(&user_id, "trader", &scopes) {
        Ok(token) => {
            Json(json!({"token":token,"token_type":"Bearer","user_id":user_id,"name":display_name,"email":email}))
                .into_response()
        }
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "AUTH_ERROR",
            &error.to_string(),
        ),
    }
}

async fn login(State(state): State<AppState>, Json(input): Json<LoginRequest>) -> Response {
    let identifier = input
        .user_id
        .as_deref()
        .or(input.email.as_deref())
        .unwrap_or_default()
        .trim();
    if identifier.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "user_id or email is required",
        );
    }
    if state.auth.mode() == AuthMode::Demo {
        // Demo bots intentionally omit a password and use ephemeral identities.
        // Interactive logins include a password, so resolve the identifier first
        // and issue the token for the canonical account user_id. This keeps an
        // unknown login from succeeding and failing later on /account/profile.
        if input.password.is_some() {
            let normalized = identifier.to_ascii_lowercase();
            let row = match sqlx::query(
                "SELECT user_id, display_name, email, status FROM auth.users WHERE user_id=$1 OR email_normalized=$2",
            )
            .bind(identifier)
            .bind(&normalized)
            .fetch_optional(&state.pool)
            .await
            {
                Ok(Some(value)) => value,
                Ok(None) => {
                    return error_response(
                        StatusCode::UNAUTHORIZED,
                        "UNAUTHENTICATED",
                        "invalid credentials",
                    )
                }
                Err(error) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "DATABASE_ERROR",
                        &error.to_string(),
                    )
                }
            };
            let status: String = row.get("status");
            if status != "ACTIVE" {
                return error_response(
                    StatusCode::FORBIDDEN,
                    "ACCOUNT_INACTIVE",
                    "account is not active",
                );
            }
            let user_id: String = row.get("user_id");
            return match state.auth.issue(&user_id, "trader") {
                Ok(token) => Json(json!({
                    "token": token,
                    "token_type": "Bearer",
                    "user_id": user_id,
                    "name": row.get::<String, _>("display_name"),
                    "email": row.get::<String, _>("email"),
                }))
                .into_response(),
                Err(error) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "AUTH_ERROR",
                    &error.to_string(),
                ),
            };
        }
        return match state.auth.issue(identifier, "trader") {
            Ok(token) => Json(json!({"token":token,"token_type":"Bearer","user_id":identifier}))
                .into_response(),
            Err(error) => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "AUTH_ERROR",
                &error.to_string(),
            ),
        };
    }
    let normalized = identifier.to_ascii_lowercase();
    let row = match sqlx::query("SELECT u.user_id, u.display_name, u.email, u.status, c.password_hash FROM auth.users u JOIN auth.credentials c ON c.subject_id=u.subject_id WHERE u.user_id=$1 OR u.email_normalized=$2")
        .bind(identifier).bind(&normalized).fetch_optional(&state.pool).await {
        Ok(Some(value)) => value,
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED", "invalid credentials"),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "DATABASE_ERROR", &error.to_string()),
    };
    let status: String = row.get("status");
    if status != "ACTIVE" {
        return error_response(
            StatusCode::FORBIDDEN,
            "ACCOUNT_INACTIVE",
            "account is not active",
        );
    }
    let password = input.password.as_deref().unwrap_or_default();
    if verify_password(password, row.get::<String, _>("password_hash").as_str()).is_err() {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHENTICATED",
            "invalid credentials",
        );
    }
    let user_id: String = row.get("user_id");
    match state.auth.issue_with_scopes(&user_id, "trader", &default_jwt_scopes()) {
        Ok(token) => Json(json!({"token":token,"token_type":"Bearer","user_id":user_id,"name":row.get::<String, _>("display_name"),"email":row.get::<String, _>("email")})).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "AUTH_ERROR", &error.to_string()),
    }
}

async fn get_profile(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authenticated_principal(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match sqlx::query(
        "SELECT user_id, display_name, email, status, created_at FROM auth.users WHERE user_id=$1",
    )
    .bind(&principal.user_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(row)) => Json(json!({
            "user_id": row.get::<String, _>("user_id"),
            "name": row.get::<String, _>("display_name"),
            "email": row.get::<String, _>("email"),
            "status": row.get::<String, _>("status"),
            "created_at": row.get::<DateTime<Utc>, _>("created_at"),
            "password": { "configured": true, "returned": false },
        }))
        .into_response(),
        Ok(None) => error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHENTICATED",
            "user account not found",
        ),
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DATABASE_ERROR",
            &error.to_string(),
        ),
    }
}

async fn load_api_keys(auth: &Authenticator, pool: &PgPool) -> anyhow::Result<()> {
    let rows = sqlx::query("SELECT k.key_hash, u.user_id, k.scopes, k.expires_at FROM auth.api_keys k JOIN auth.users u ON u.subject_id=k.subject_id WHERE k.revoked_at IS NULL AND (k.expires_at IS NULL OR k.expires_at > now())")
        .fetch_all(pool).await?;
    for row in rows {
        let expires_at = row
            .try_get::<Option<DateTime<Utc>>, _>("expires_at")?
            .map(|value| value.timestamp());
        auth.register_api_key(
            row.try_get("key_hash")?,
            row.try_get("user_id")?,
            row.try_get("scopes")?,
            expires_at,
        );
    }
    Ok(())
}

async fn list_api_keys(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authenticated_principal(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let user_id = principal.user_id.clone();
    match sqlx::query("SELECT k.key_id, k.key_prefix, k.name, k.scopes, k.created_at, k.expires_at, k.last_used_at, k.revoked_at FROM auth.api_keys k JOIN auth.users u ON u.subject_id=k.subject_id WHERE u.user_id=$1 ORDER BY k.created_at DESC")
        .bind(&user_id).fetch_all(&state.pool).await {
        Ok(rows) => Json(json!({"api_keys": rows.iter().map(|row| json!({"key_id":row.get::<Uuid,_>("key_id"),"key_prefix":row.get::<String,_>("key_prefix"),"name":row.get::<String,_>("name"),"scopes":row.get::<Vec<String>,_>("scopes"),"created_at":row.get::<DateTime<Utc>,_>("created_at"),"expires_at":row.get::<Option<DateTime<Utc>>,_>("expires_at"),"last_used_at":row.get::<Option<DateTime<Utc>>,_>("last_used_at"),"revoked_at":row.get::<Option<DateTime<Utc>>,_>("revoked_at")})).collect::<Vec<_>>() })).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "DATABASE_ERROR", &error.to_string()),
    }
}

async fn create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ApiKeyCreateRequest>,
) -> Response {
    let principal = match authenticated_principal(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let user_id = principal.user_id.clone();
    let name = input.name.trim();
    if name.is_empty() || name.len() > 80 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "name must be 1-80 characters",
        );
    }
    let scopes = match validate_api_key_scopes(input.scopes.unwrap_or_else(default_api_key_scopes))
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    if principal.method == AuthMethod::ApiKey
        && scopes.iter().any(|scope| {
            !principal
                .scopes
                .iter()
                .any(|owned| owned == "*" || owned == scope)
        })
    {
        return error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "an api key cannot grant scopes it does not own",
        );
    }
    let expires_at = match input.expires_at.as_deref() {
        Some(value) => match DateTime::parse_from_rfc3339(value) {
            Ok(value) => Some(value.with_timezone(&Utc)),
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
    let (raw, prefix, hash) = generate_api_key();
    let row =
        match sqlx::query("SELECT subject_id FROM auth.users WHERE user_id=$1 AND status='ACTIVE'")
            .bind(&user_id)
            .fetch_optional(&state.pool)
            .await
        {
            Ok(Some(value)) => value,
            Ok(None) => {
                return error_response(
                    StatusCode::UNAUTHORIZED,
                    "UNAUTHENTICATED",
                    "user account not found",
                )
            }
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "DATABASE_ERROR",
                    &error.to_string(),
                )
            }
        };
    let subject_id: Uuid = row.get("subject_id");
    let inserted = sqlx::query("INSERT INTO auth.api_keys (subject_id,key_prefix,key_hash,name,scopes,expires_at) VALUES ($1,$2,$3,$4,$5,$6) RETURNING key_id,created_at,expires_at")
        .bind(subject_id).bind(&prefix).bind(&hash).bind(name).bind(&scopes).bind(expires_at).fetch_one(&state.pool).await;
    match inserted {
        Ok(row) => {
            state.auth.register_api_key(
                hash,
                user_id.clone(),
                scopes.clone(),
                expires_at.map(|value| value.timestamp()),
            );
            Json(json!({"key_id":row.get::<Uuid,_>("key_id"),"name":name,"key":raw,"key_prefix":prefix,"scopes":scopes,"created_at":row.get::<DateTime<Utc>,_>("created_at"),"expires_at":row.get::<Option<DateTime<Utc>>,_>("expires_at")})).into_response()
        }
        Err(error) if is_unique_violation(&error) => error_response(
            StatusCode::CONFLICT,
            "ALREADY_EXISTS",
            "api key collision; retry",
        ),
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DATABASE_ERROR",
            &error.to_string(),
        ),
    }
}

async fn revoke_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<Uuid>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let row = match sqlx::query("UPDATE auth.api_keys k SET revoked_at=now() FROM auth.users u WHERE k.key_id=$1 AND k.subject_id=u.subject_id AND u.user_id=$2 AND k.revoked_at IS NULL RETURNING k.key_hash")
        .bind(key_id).bind(&user_id).fetch_optional(&state.pool).await {
        Ok(Some(value)) => value,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "api key not found"),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "DATABASE_ERROR", &error.to_string()),
    };
    state.auth.revoke_api_key(&row.get::<String, _>("key_hash"));
    Json(json!({"key_id":key_id,"revoked":true})).into_response()
}

fn default_jwt_scopes() -> Vec<String> {
    vec![
        "markets:read".into(),
        "account:read".into(),
        "orders:read".into(),
        "fills:read".into(),
        "trading:write".into(),
        "websocket:read".into(),
    ]
}
fn default_api_key_scopes() -> Vec<String> {
    vec![
        "markets:read".into(),
        "account:read".into(),
        "orders:read".into(),
        "fills:read".into(),
    ]
}
fn validate_api_key_scopes(scopes: Vec<String>) -> Result<Vec<String>, Response> {
    let allowed = [
        "markets:read",
        "account:read",
        "orders:read",
        "fills:read",
        "trading:write",
        "websocket:read",
    ];
    if scopes.is_empty()
        || scopes
            .iter()
            .any(|scope| !allowed.contains(&scope.as_str()))
    {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "unsupported api key scope",
        ));
    }
    Ok(scopes)
}
fn valid_user_id(value: &str) -> bool {
    (3..=32).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
fn valid_email(value: &str) -> bool {
    value.len() <= 254
        && value.contains('@')
        && value.split('@').count() == 2
        && value
            .rsplit('@')
            .next()
            .is_some_and(|domain| domain.contains('.'))
}
fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.code().as_deref() == Some("23505"))
}

async fn list_markets(State(state): State<AppState>, Query(query): Query<MarketQuery>) -> Response {
    let expected_resolution_from = match query_timestamp(
        query.expected_resolution_from.as_deref(),
        "expected_resolution_from",
    ) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let expected_resolution_to = match query_timestamp(
        query.expected_resolution_to.as_deref(),
        "expected_resolution_to",
    ) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let kind = match query.kind.as_deref() {
        None => 0,
        Some(value) => match kind_value(value) {
            Some(value) => value,
            None => {
                return error_response(StatusCode::BAD_REQUEST, "INVALID_ARGUMENT", "invalid kind")
            }
        },
    };
    let mut client = state.refdata;
    match rpc(client.list_contracts(ListContractsRequest {
        state: query.state.as_deref().and_then(state_value).unwrap_or(0),
        series_ticker: query.series_ticker.unwrap_or_default(),
        kind,
        event_ticker: query.event_ticker.unwrap_or_default(),
        underlying: query.underlying.unwrap_or_default(),
        category: query.category.unwrap_or_default(),
        expected_resolution_from,
        expected_resolution_to,
        limit: query.limit.unwrap_or(50).clamp(1,500),
        cursor: query.cursor.unwrap_or_default(),
    })).await {
        Ok(response) => Json(json!({"contracts":response.contracts.iter().map(contract_json).collect::<Vec<_>>(),"next_cursor":response.next_cursor})).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn list_series(
    State(state): State<AppState>,
    Query(query): Query<DiscoveryQuery>,
) -> Response {
    let mut client = state.refdata;
    match rpc(client.list_series(ListSeriesRequest {
        limit: query.limit.unwrap_or(100).clamp(1, 200),
        cursor: query.cursor.unwrap_or_default(),
    }))
    .await
    {
        Ok(response) => Json(json!({
            "series": response.series.iter().map(series_json).collect::<Vec<_>>(),
            "next_cursor": response.next_cursor,
        }))
        .into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<DiscoveryQuery>,
) -> Response {
    let expected_resolution_from = match query_timestamp(
        query.expected_resolution_from.as_deref(),
        "expected_resolution_from",
    ) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let expected_resolution_to = match query_timestamp(
        query.expected_resolution_to.as_deref(),
        "expected_resolution_to",
    ) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.refdata;
    match rpc(client.list_events(ListEventsRequest {
        series_ticker: query.series_ticker.unwrap_or_default(),
        expected_resolution_from,
        expected_resolution_to,
        limit: query.limit.unwrap_or(100).clamp(1, 200),
        cursor: query.cursor.unwrap_or_default(),
    }))
    .await
    {
        Ok(response) => Json(json!({
            "events": response.events.iter().map(event_json).collect::<Vec<_>>(),
            "next_cursor": response.next_cursor,
        }))
        .into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn get_event(State(state): State<AppState>, Path(event_ticker): Path<String>) -> Response {
    let mut client = state.refdata;
    match rpc(client.get_event(sarvex_contracts::sarvex::v1::GetEventRequest { event_ticker }))
        .await
    {
        Ok(event) => Json(event_json(&event)).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn get_resolution(
    State(state): State<AppState>,
    Path(event_ticker): Path<String>,
) -> Response {
    let mut client = state.oracle;
    match rpc(client.get_resolution(GetResolutionRequest { event_ticker })).await {
        Ok(resolution) => Json(resolution_json(&resolution)).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn get_settlement(State(state): State<AppState>, Path(ticker): Path<String>) -> Response {
    let mut client = state.settlement;
    match rpc(client.get_settlement(GetSettlementRequest { ticker })).await {
        Ok(settlement) => Json(settlement_json(&settlement)).into_response(),
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
    match rpc(client.list_fills(ListFillsRequest { ticker, from_global_seq: query.from_global_seq.unwrap_or(0), to_global_seq: query.to_global_seq.unwrap_or(0), limit: query.limit.unwrap_or(100).clamp(1,500), cursor: query.cursor.unwrap_or_default(), user_id: String::new(), order_id: String::new(), from_time: None, to_time: None })).await {
        Ok(response) => Json(json!({"fills":response.fills.iter().map(fill_record_json).collect::<Vec<_>>(),"next_cursor":response.next_cursor})).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn list_account_fills(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FillQuery>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let from_time = match query_timestamp(query.from_time.as_deref(), "from_time") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let to_time = match query_timestamp(query.to_time.as_deref(), "to_time") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.orders;
    let request = ListFillsRequest {
        ticker: query.ticker.unwrap_or_default(),
        from_global_seq: query.from_global_seq.unwrap_or(0),
        to_global_seq: query.to_global_seq.unwrap_or(0),
        limit: query.limit.unwrap_or(100).clamp(1, 500),
        cursor: query.cursor.unwrap_or_default(),
        user_id: user_id.clone(),
        order_id: query.order_id.unwrap_or_default(),
        from_time,
        to_time,
    };
    match rpc(client.list_fills(request)).await {
        Ok(response) => Json(json!({
            "fills": response
                .fills
                .iter()
                .filter_map(|fill| private_fill_record_json(fill, &user_id))
                .collect::<Vec<_>>(),
            "next_cursor": response.next_cursor,
        }))
        .into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn submit_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<OrderInput>, JsonRejection>,
) -> Response {
    let input = match body {
        Ok(Json(input)) => input,
        Err(error) => {
            let status = match error {
                JsonRejection::JsonSyntaxError(_) => StatusCode::BAD_REQUEST,
                JsonRejection::JsonDataError(_) => StatusCode::UNPROCESSABLE_ENTITY,
                _ => StatusCode::BAD_REQUEST,
            };
            return error_response(status, "INVALID_ARGUMENT", "invalid JSON request body");
        }
    };
    let user_id = match authenticated_user_with_scope(&headers, &state.auth, "trading:write") {
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
    if input.client_order_id.len() > 128 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "client_order_id must be 128 characters or fewer",
        );
    }
    if input.side.trim() != input.side || input.side != input.side.to_ascii_uppercase() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "side must use an uppercase enum value",
        );
    }
    if input.action.trim() != input.action || input.action != input.action.to_ascii_uppercase() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "action must use an uppercase enum value",
        );
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
    let order_type = input
        .order_type
        .as_deref()
        .or(input.type_alias.as_deref())
        .unwrap_or("LIMIT");
    if !matches!(order_type, "LIMIT" | "MARKET") {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "invalid order_type",
        );
    }
    let market = order_type == "MARKET";
    if !input.tif.is_empty() && !matches!(input.tif.as_str(), "GTC" | "IOC" | "FOK") {
        return error_response(StatusCode::BAD_REQUEST, "INVALID_ARGUMENT", "invalid tif");
    }
    if !input.stp.is_empty()
        && !matches!(
            input.stp.as_str(),
            "UNSPECIFIED" | "TAKER_AT_CROSS" | "MAKER"
        )
    {
        return error_response(StatusCode::BAD_REQUEST, "INVALID_ARGUMENT", "invalid stp");
    }
    let contract = match state
        .refdata
        .clone()
        .get_contract(GetContractRequest {
            ticker: input.ticker.clone(),
        })
        .await
    {
        Ok(contract) => contract.into_inner(),
        Err(error) if error.code() == Code::NotFound => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENT",
                "unknown ticker",
            )
        }
        Err(error) => return grpc_error(error),
    };
    let side_is_binary = side == Side::Yes as i32 || side == Side::No as i32;
    let side_is_futures = side == Side::Long as i32 || side == Side::Short as i32;
    let valid_side = match ContractKind::try_from(contract.kind).ok() {
        Some(ContractKind::Binary) => side_is_binary,
        Some(ContractKind::Scalar) => side_is_futures,
        _ => false,
    };
    if !valid_side {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "side does not match the contract kind",
        );
    }
    let price_ticks = if market {
        let snapshot = match state.me.get_book_snapshot(input.ticker.clone(), 1).await {
            Ok(snapshot) => snapshot,
            Err(error) => return me_core_error(error),
        };
        match market_protection_price(side, action, &contract, &snapshot) {
            Ok(price) => price,
            Err(reason) => {
                return error_response(StatusCode::BAD_REQUEST, "MARKET_NOT_AVAILABLE", &reason)
            }
        }
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
    if expires_at
        .as_ref()
        .and_then(|value| DateTime::<Utc>::from_timestamp(value.seconds, value.nanos as u32))
        .is_some_and(|value| value <= Utc::now())
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "expires_at must be in the future",
        );
    }
    let mut client = state.orders;
    let (value, response_status) = match rpc(client.submit_order(SubmitOrderRequest {
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
            let status = order_rejection_status(&response.reject_code);
            (
                json!({"order":response.order.as_ref().map(order_json),"fills":response.fills.iter().map(fill_json).collect::<Vec<_>>(),"reject_code":response.reject_code,"reject_reason":response.reject_reason}),
                status,
            )
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
        response_status,
    )
    .await
    {
        return response;
    }
    (response_status, Json(value)).into_response()
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
    let user_id = match authenticated_user_with_scope(&headers, &state.auth, "trading:write") {
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

async fn get_account_risk(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };

    let mut ledger = state.ledger;
    let balance = match rpc(ledger.get_balance(GetBalanceRequest {
        user_id: user_id.clone(),
    }))
    .await
    {
        Ok(value) => value,
        Err(error) => return grpc_error(error),
    };

    let mut positions = state.positions;
    let positions_response = match rpc(positions.list_positions(ListPositionsRequest {
        user_id: user_id.clone(),
        include_closed: false,
        limit: 500,
        cursor: String::new(),
    }))
    .await
    {
        Ok(value) => value,
        Err(error) => return grpc_error(error),
    };

    let mut orders = state.orders;
    let orders_response = match rpc(orders.list_orders(ListOrdersRequest {
        user_id: user_id.clone(),
        ticker: String::new(),
        status: 0,
        limit: 500,
        cursor: String::new(),
    }))
    .await
    {
        Ok(value) => value,
        Err(error) => return grpc_error(error),
    };

    let mut risk = state.risk;
    let limits = match rpc(risk.get_user_limits(GetUserLimitsRequest {
        user_id: user_id.clone(),
    }))
    .await
    {
        Ok(value) => value,
        Err(error) => return grpc_error(error),
    };

    let realized_pnl = match checked_position_sum(
        positions_response
            .positions
            .iter()
            .map(|position| position.realized_pnl_micro_usdc),
    ) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let unrealized_pnl = match checked_position_sum(
        positions_response
            .positions
            .iter()
            .map(|position| position.unrealized_pnl_micro_usdc),
    ) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let position_cost = match checked_position_cost(&positions_response.positions) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let equity = match balance.total_micro_usdc.checked_add(unrealized_pnl) {
        Some(value) => value,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "NUMERIC_OVERFLOW",
                "account equity exceeds supported range",
            )
        }
    };
    let open_orders = orders_response
        .orders
        .iter()
        .filter(|order| {
            matches!(
                OrderStatus::try_from(order.status).ok(),
                Some(OrderStatus::Open | OrderStatus::Partial)
            )
        })
        .count();
    let as_of_global_seq = positions_response
        .positions
        .iter()
        .map(|position| position.last_global_seq)
        .max()
        .unwrap_or_default();

    Json(json!({
        "user_id": user_id,
        "cash_micro_usdc": balance.cash_micro_usdc,
        "held_micro_usdc": balance.held_micro_usdc,
        "total_micro_usdc": balance.total_micro_usdc,
        "available_collateral_micro_usdc": balance.cash_micro_usdc,
        "equity_micro_usdc": equity,
        "position_cost_micro_usdc": position_cost,
        "realized_pnl_micro_usdc": realized_pnl,
        "unrealized_pnl_micro_usdc": unrealized_pnl,
        "open_positions": positions_response.positions.len(),
        "open_orders": open_orders,
        "as_of_global_seq": as_of_global_seq,
        "limits": user_limits_json(&limits),
        "calculation_basis": {
            "available_collateral": "ledger cash balance; held collateral is reported separately",
            "equity": "ledger total balance plus position unrealized P&L",
            "position_cost": "sum of absolute net quantity multiplied by absolute average cost"
        }
    }))
    .into_response()
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

async fn list_positions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PositionQuery>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.positions;
    match rpc(client.list_positions(ListPositionsRequest {
        user_id,
        include_closed: query.include_closed.unwrap_or(false),
        limit: query.limit.unwrap_or(100).clamp(1, 500),
        cursor: query.cursor.unwrap_or_default(),
    })).await {
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
        Ok(value) => Json(json!({"ticker":value.ticker,"total_open_long":value.total_open_long,"total_open_short":value.total_open_short,"as_of_global_seq":value.as_of_global_seq,"as_of":timestamp_json(value.as_of.as_ref())})).into_response(), Err(error) => grpc_error(error),
    }
}

async fn create_rfq(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RfqInput>,
) -> Response {
    let user_id = match authenticated_user_with_scope(&headers, &state.auth, "trading:write") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let side = match side_value(&input.side) {
        Some(value) => value,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENT",
                "invalid RFQ side",
            )
        }
    };
    let action = match action_value(&input.action) {
        Some(value) => value,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENT",
                "invalid RFQ action",
            )
        }
    };
    let expires_at = match parse_timestamp(&input.expires_at) {
        Ok(Some(value)) => value,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENT",
                "invalid RFQ expires_at",
            )
        }
    };
    let key = match required_header(&headers, "idempotency-key") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let body = json!(&input);
    if let Some(response) =
        match replay_idempotency(&state.pool, &user_id, "POST", "/v1/rfqs", &key, &body).await {
            Ok(value) => value,
            Err(response) => return response,
        }
    {
        return response;
    }
    let mut client = state.rfq.clone();
    let value = match rpc(client.create_rfq(CreateRfqRequest {
        creator_user_id: user_id.clone(),
        client_rfq_id: input.client_rfq_id,
        ticker: input.ticker,
        side,
        action,
        requested_count: input.requested_count,
        expires_at: Some(expires_at),
        idempotency_key: key.clone(),
    }))
    .await
    {
        Ok(rfq) => json!({"rfq": rfq_json(&rfq)}),
        Err(error) => return grpc_error(error),
    };
    if let Err(response) = store_idempotency(
        &state.pool,
        &user_id,
        "POST",
        "/v1/rfqs",
        &key,
        &body,
        &value,
        StatusCode::OK,
    )
    .await
    {
        return response;
    }
    Json(value).into_response()
}

async fn get_rfq(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(rfq_id): Path<String>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.rfq.clone();
    match rpc(client.get_rfq(GetRfqRequest { user_id, rfq_id })).await {
        Ok(rfq) => Json(rfq_json(&rfq)).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn list_rfq_quotes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(rfq_id): Path<String>,
    Query(query): Query<OrderQuery>,
) -> Response {
    let user_id = match authenticated_user(&headers, &state.auth) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut client = state.rfq.clone();
    match rpc(client.list_quotes(ListQuotesRequest { user_id, rfq_id, limit: query.limit.unwrap_or(100).clamp(1, 500), cursor: query.cursor.unwrap_or_default() })).await {
        Ok(response) => Json(json!({"quotes": response.quotes.iter().map(rfq_quote_json).collect::<Vec<_>>(), "next_cursor": response.next_cursor})).into_response(),
        Err(error) => grpc_error(error),
    }
}

async fn submit_rfq_quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(rfq_id): Path<String>,
    Json(input): Json<RfqQuoteInput>,
) -> Response {
    let user_id = match authenticated_user_with_scope(&headers, &state.auth, "trading:write") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let expires_at = match parse_timestamp(&input.expires_at) {
        Ok(Some(value)) => value,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENT",
                "invalid quote expires_at",
            )
        }
    };
    let key = match required_header(&headers, "idempotency-key") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let body = json!({"rfq_id":rfq_id,"quote":input});
    let path = format!("/v1/rfqs/{rfq_id}/quotes");
    if let Some(response) =
        match replay_idempotency(&state.pool, &user_id, "POST", &path, &key, &body).await {
            Ok(value) => value,
            Err(response) => return response,
        }
    {
        return response;
    }
    let mut client = state.rfq.clone();
    let value = match rpc(client.submit_quote(SubmitQuoteRequest {
        maker_user_id: user_id.clone(),
        quote_id: input.quote_id,
        rfq_id: rfq_id.clone(),
        bid_price_ticks: input.bid_price_ticks,
        offer_price_ticks: input.offer_price_ticks,
        available_count: input.available_count,
        expires_at: Some(expires_at),
        idempotency_key: key.clone(),
    }))
    .await
    {
        Ok(quote) => json!({"quote":rfq_quote_json(&quote)}),
        Err(error) => return grpc_error(error),
    };
    if let Err(response) = store_idempotency(
        &state.pool,
        &user_id,
        "POST",
        &path,
        &key,
        &body,
        &value,
        StatusCode::OK,
    )
    .await
    {
        return response;
    }
    Json(value).into_response()
}

async fn cancel_rfq(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(rfq_id): Path<String>,
) -> Response {
    let user_id = match authenticated_user_with_scope(&headers, &state.auth, "trading:write") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let key = match required_header(&headers, "idempotency-key") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let path = format!("/v1/rfqs/{rfq_id}/cancel");
    let body = json!({"rfq_id":rfq_id});
    if let Some(response) =
        match replay_idempotency(&state.pool, &user_id, "POST", &path, &key, &body).await {
            Ok(value) => value,
            Err(response) => return response,
        }
    {
        return response;
    }
    let mut client = state.rfq.clone();
    let value = match rpc(client.cancel_rfq(CancelRfqRequest {
        creator_user_id: user_id.clone(),
        rfq_id: rfq_id.clone(),
        idempotency_key: key.clone(),
    }))
    .await
    {
        Ok(rfq) => json!({"rfq":rfq_json(&rfq)}),
        Err(error) => return grpc_error(error),
    };
    if let Err(response) = store_idempotency(
        &state.pool,
        &user_id,
        "POST",
        &path,
        &key,
        &body,
        &value,
        StatusCode::OK,
    )
    .await
    {
        return response;
    }
    Json(value).into_response()
}

async fn cancel_rfq_quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((rfq_id, quote_id)): Path<(String, String)>,
) -> Response {
    let user_id = match authenticated_user_with_scope(&headers, &state.auth, "trading:write") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let key = match required_header(&headers, "idempotency-key") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let path = format!("/v1/rfqs/{rfq_id}/quotes/{quote_id}/cancel");
    let body = json!({"rfq_id":rfq_id,"quote_id":quote_id});
    if let Some(response) =
        match replay_idempotency(&state.pool, &user_id, "POST", &path, &key, &body).await {
            Ok(value) => value,
            Err(response) => return response,
        }
    {
        return response;
    }
    let mut client = state.rfq.clone();
    let value = match rpc(client.cancel_quote(CancelQuoteRequest {
        maker_user_id: user_id.clone(),
        quote_id: quote_id.clone(),
        idempotency_key: key.clone(),
    }))
    .await
    {
        Ok(quote) => json!({"quote":rfq_quote_json(&quote)}),
        Err(error) => return grpc_error(error),
    };
    if let Err(response) = store_idempotency(
        &state.pool,
        &user_id,
        "POST",
        &path,
        &key,
        &body,
        &value,
        StatusCode::OK,
    )
    .await
    {
        return response;
    }
    Json(value).into_response()
}

async fn accept_rfq_quote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((rfq_id, quote_id)): Path<(String, String)>,
) -> Response {
    let user_id = match authenticated_user_with_scope(&headers, &state.auth, "trading:write") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let key = match required_header(&headers, "idempotency-key") {
        Ok(value) => value,
        Err(response) => return response,
    };
    let path = format!("/v1/rfqs/{rfq_id}/quotes/{quote_id}/accept");
    let body = json!({"rfq_id":rfq_id,"quote_id":quote_id});
    if let Some(response) =
        match replay_idempotency(&state.pool, &user_id, "POST", &path, &key, &body).await {
            Ok(value) => value,
            Err(response) => return response,
        }
    {
        return response;
    }
    let mut client = state.rfq.clone();
    let value = match rpc(client.accept_quote(AcceptQuoteRequest {
        creator_user_id: user_id.clone(),
        rfq_id: rfq_id.clone(),
        quote_id: quote_id.clone(),
        idempotency_key: key.clone(),
    }))
    .await
    {
        Ok(rfq) => json!({"rfq":rfq_json(&rfq),"execution_pending":true}),
        Err(error) => return grpc_error(error),
    };
    if let Err(response) = store_idempotency(
        &state.pool,
        &user_id,
        "POST",
        &path,
        &key,
        &body,
        &value,
        StatusCode::OK,
    )
    .await
    {
        return response;
    }
    Json(value).into_response()
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
    authenticated_principal(headers, auth).map(|principal| principal.user_id)
}

fn authenticated_user_with_scope(
    headers: &HeaderMap,
    auth: &Authenticator,
    required_scope: &str,
) -> Result<String, Response> {
    let principal = authenticated_principal(headers, auth)?;
    if principal
        .scopes
        .iter()
        .any(|scope| scope == "*" || scope == required_scope)
    {
        Ok(principal.user_id)
    } else {
        Err(error_response(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "credential does not have the required scope",
        ))
    }
}

fn authenticated_principal(
    headers: &HeaderMap,
    auth: &Authenticator,
) -> Result<sarvex_auth::Principal, Response> {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let api_key = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok());
    auth.verify_request(authorization, api_key)
        .map_err(|_error| {
            error_response(
                StatusCode::UNAUTHORIZED,
                "UNAUTHENTICATED",
                "invalid or missing credentials",
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
    let inserted = sqlx::query("INSERT INTO gateway.idempotency_records (user_id, method, request_path, idempotency_key, request_hash, operation_id, status, response_status, response_body) VALUES ($1,$2,$3,$4,$5,'op_' || md5($1 || ':' || $2 || ':' || $3 || ':' || $4),'IN_PROGRESS',202,NULL) ON CONFLICT (user_id, method, request_path, idempotency_key) DO NOTHING RETURNING operation_id")
        .bind(user_id)
        .bind(method)
        .bind(path)
        .bind(key)
        .bind(&hash)
        .fetch_optional(pool)
        .await
        .map_err(|error| error_response(StatusCode::SERVICE_UNAVAILABLE, "IDEMPOTENCY_STORE_UNAVAILABLE", &error.to_string()))?;
    if inserted.is_some() {
        return Ok(None);
    }
    let row = sqlx::query("SELECT request_hash, status, response_status, response_body FROM gateway.idempotency_records WHERE user_id=$1 AND method=$2 AND request_path=$3 AND idempotency_key=$4 AND expires_at > now()")
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
    let operation_status: String = row.try_get("status").map_err(|error| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "IDEMPOTENCY_STORE_ERROR",
            &error.to_string(),
        )
    })?;
    if operation_status != "COMPLETED" {
        return Err(error_response(
            StatusCode::CONFLICT,
            "OPERATION_IN_PROGRESS",
            "the original operation is still being reconciled; retry with the same key",
        ));
    }
    let status: i32 = row.try_get("response_status").map_err(|error| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "IDEMPOTENCY_STORE_ERROR",
            &error.to_string(),
        )
    })?;
    let body: Value = row
        .try_get::<Option<Value>, _>("response_body")
        .map_err(|error| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "IDEMPOTENCY_STORE_ERROR",
                &error.to_string(),
            )
        })?
        .ok_or_else(|| {
            error_response(
                StatusCode::CONFLICT,
                "OPERATION_IN_PROGRESS",
                "the original operation has no stored response yet",
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
    sqlx::query("UPDATE gateway.idempotency_records SET status='COMPLETED', response_status=$6, response_body=$7, updated_at=now() WHERE user_id=$1 AND method=$2 AND request_path=$3 AND idempotency_key=$4 AND request_hash=$5")
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

fn market_protection_price(
    _side: i32,
    action: i32,
    contract: &Contract,
    snapshot: &BookSnapshot,
) -> Result<i64, String> {
    let action_is_buy = action == Action::Buy as i32;
    let best = if action_is_buy {
        snapshot.asks.first().map(|level| level.price_ticks)
    } else {
        snapshot.bids.first().map(|level| level.price_ticks)
    }
    .filter(|price| *price > 0)
    .ok_or_else(|| "market order requires a live opposite-side quote".to_owned())?;
    let min = contract.min_price_ticks.max(contract.lower_bound_ticks);
    let max = if contract.max_price_ticks > 0 {
        contract
            .max_price_ticks
            .min(contract.upper_bound_ticks.max(contract.max_price_ticks))
    } else {
        contract.upper_bound_ticks
    };
    let width = max
        .checked_sub(min)
        .filter(|value| *value > 0)
        .ok_or_else(|| "contract price band is invalid".to_owned())?;
    let tick = contract.tick_size.max(1);
    let protection = ((width + 19) / 20).max(tick);
    let raw = if action_is_buy {
        best.saturating_add(protection)
    } else {
        best.saturating_sub(protection)
    };
    let aligned = if action_is_buy {
        ((raw + tick - 1) / tick) * tick
    } else {
        (raw / tick) * tick
    };
    Ok(aligned.clamp(min, max))
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
fn kind_value(value: &str) -> Option<i32> {
    match value.to_ascii_uppercase().as_str() {
        "BINARY" => Some(ContractKind::Binary as i32),
        "FUTURE" | "FUTURES" | "SCALAR" => Some(ContractKind::Scalar as i32),
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
fn query_timestamp(value: Option<&str>, field: &str) -> Result<Option<Timestamp>, Response> {
    match value {
        Some(value) => parse_timestamp(value).map_err(|_| {
            error_response(
                StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENT",
                &format!("{field} must be RFC3339"),
            )
        }),
        None => Ok(None),
    }
}

fn contract_json(contract: &Contract) -> Value {
    json!({"ticker":contract.ticker,"event_ticker":contract.event_ticker,"series_ticker":contract.series_ticker,"kind":contract.kind,"question":contract.question,"underlying":contract.underlying,"tick_size":contract.tick_size,"min_price_ticks":contract.min_price_ticks,"max_price_ticks":contract.max_price_ticks,"lower_bound_ticks":contract.lower_bound_ticks,"upper_bound_ticks":contract.upper_bound_ticks,"multiplier_micro_usdc":contract.multiplier_micro_usdc,"divider":contract.divider,"multiplier_micro_per_display_unit":contract.multiplier_micro_per_display_unit,"tick_value_micro":contract.tick_value_micro,"max_order_size":contract.max_order_size,"position_limit_per_user":contract.position_limit_per_user,"state":contract.state,"listed_at":timestamp_json(contract.listed_at.as_ref()),"open_at":timestamp_json(contract.open_at.as_ref()),"close_at":timestamp_json(contract.close_at.as_ref()),"expected_resolution_at":timestamp_json(contract.expected_resolution_at.as_ref()),"settlement_source":contract.settlement_source,"oracle_policy":contract.oracle_policy,"settlement_rule":contract.settlement_rule.as_ref().map(struct_json),"close_global_seq":contract.close_global_seq})
}
fn series_json(series: &Series) -> Value {
    json!({
        "series_ticker": series.series_ticker,
        "title": series.title,
        "description": series.description,
    })
}
fn event_json(event: &Event) -> Value {
    json!({
        "event_ticker": event.event_ticker,
        "series_ticker": event.series_ticker,
        "title": event.title,
        "description": event.description,
        "expected_resolution_at": timestamp_json(event.expected_resolution_at.as_ref()),
    })
}
fn resolution_json(resolution: &Resolution) -> Value {
    json!({
        "event_ticker": resolution.event_ticker,
        "numeric_value": resolution.numeric_value,
        "categorical_value": resolution.categorical_value,
        "status": enum_name(sarvex_contracts::sarvex::v1::ResolutionStatus::try_from(resolution.status).ok()),
        "attestations": resolution.attestations.iter().map(|attestation| json!({
            "attestor_id": attestation.attestor_id,
            "source": attestation.source,
            "numeric_value": attestation.numeric_value,
            "categorical_value": attestation.categorical_value,
            "observed_at": timestamp_json(attestation.observed_at.as_ref()),
        })).collect::<Vec<_>>(),
        "proposed_at": timestamp_json(resolution.proposed_at.as_ref()),
        "finalized_at": timestamp_json(resolution.finalized_at.as_ref()),
    })
}
fn settlement_json(settlement: &SettlementResult) -> Value {
    json!({
        "ticker": settlement.ticker,
        "settled_at": timestamp_json(settlement.settled_at.as_ref()),
        "winner_payout_per_contract_micro_usdc": settlement.winner_payout_per_contract_micro_usdc,
        "total_payout_micro_usdc": settlement.total_payout_micro_usdc,
        "positions_settled": settlement.positions_settled,
    })
}
fn order_json(order: &Order) -> Value {
    json!({"order_id":order.order_id,"client_order_id":order.client_order_id,"user_id":order.user_id,"ticker":order.ticker,"side":enum_name(Side::try_from(order.side).ok()),"action":enum_name(Action::try_from(order.action).ok()),"price_ticks":order.price_ticks,"count":order.count,"filled_count":order.filled_count,"remaining_count":order.remaining_count,"cancelled_count":order.cancelled_count,"expired_count":order.expired_count,"tif":enum_name(TimeInForce::try_from(order.tif).ok()),"post_only":order.post_only,"reduce_only":order.reduce_only,"stp":enum_name(SelfTradePreventionType::try_from(order.stp).ok()),"status":enum_name(OrderStatus::try_from(order.status).ok()),"created_at":timestamp_json(order.created_at.as_ref()),"updated_at":timestamp_json(order.updated_at.as_ref()),"expires_at":timestamp_json(order.expires_at.as_ref()),"hold_id":order.hold_id,"avg_fill_price_ticks":order.avg_fill_price_ticks})
}
fn rfq_json(rfq: &Rfq) -> Value {
    json!({
        "rfq_id": rfq.rfq_id,
        "client_rfq_id": rfq.client_rfq_id,
        "creator_user_id": rfq.creator_user_id,
        "ticker": rfq.ticker,
        "side": enum_name(Side::try_from(rfq.side).ok()),
        "action": enum_name(Action::try_from(rfq.action).ok()),
        "requested_count": rfq.requested_count,
        "expires_at": timestamp_json(rfq.expires_at.as_ref()),
        "status": enum_name(RfqStatus::try_from(rfq.status).ok()),
        "accepted_quote_id": rfq.accepted_quote_id,
        "created_at": timestamp_json(rfq.created_at.as_ref()),
        "updated_at": timestamp_json(rfq.updated_at.as_ref()),
    })
}
fn rfq_quote_json(quote: &RfqQuote) -> Value {
    json!({
        "quote_id": quote.quote_id,
        "rfq_id": quote.rfq_id,
        "maker_user_id": quote.maker_user_id,
        "bid_price_ticks": quote.bid_price_ticks,
        "offer_price_ticks": quote.offer_price_ticks,
        "available_count": quote.available_count,
        "expires_at": timestamp_json(quote.expires_at.as_ref()),
        "status": enum_name(RfqQuoteStatus::try_from(quote.status).ok()),
        "created_at": timestamp_json(quote.created_at.as_ref()),
        "updated_at": timestamp_json(quote.updated_at.as_ref()),
    })
}
fn fill_json(fill: &Fill) -> Value {
    json!({"fill_id":fill.fill_id,"order_id":fill.order_id,"ticker":fill.ticker,"price_ticks":fill.price_ticks,"count":fill.count,"aggressor_side":enum_name(Side::try_from(fill.aggressor_side).ok()),"fee_micro_usdc":fill.fee_micro_usdc,"ts":timestamp_json(fill.ts.as_ref()),"seq":fill.seq})
}
fn fill_record_json(fill: &sarvex_contracts::sarvex::v1::FillRecord) -> Value {
    json!({"fill_id":fill.fill_id,"ticker":fill.ticker,"global_seq":fill.global_seq,"contract_seq":fill.contract_seq,"maker_order_id":fill.maker_order_id,"taker_order_id":fill.taker_order_id,"maker_user_id":fill.maker_user_id,"taker_user_id":fill.taker_user_id,"maker_side":enum_name(Side::try_from(fill.maker_side).ok()),"maker_action":enum_name(Action::try_from(fill.maker_action).ok()),"taker_side":enum_name(Side::try_from(fill.taker_side).ok()),"taker_action":enum_name(Action::try_from(fill.taker_action).ok()),"price_ticks":fill.price_ticks,"count":fill.count,"aggressor_side":enum_name(Side::try_from(fill.aggressor_side).ok()),"maker_fee_micro_usdc":fill.maker_fee_micro_usdc,"taker_fee_micro_usdc":fill.taker_fee_micro_usdc,"ts":timestamp_json(fill.ts.as_ref())})
}
fn private_fill_record_json(
    fill: &sarvex_contracts::sarvex::v1::FillRecord,
    user_id: &str,
) -> Option<Value> {
    let (role, order_id, side, action, fee_micro_usdc) = if fill.maker_user_id == user_id {
        (
            "MAKER",
            &fill.maker_order_id,
            fill.maker_side,
            fill.maker_action,
            fill.maker_fee_micro_usdc,
        )
    } else if fill.taker_user_id == user_id {
        (
            "TAKER",
            &fill.taker_order_id,
            fill.taker_side,
            fill.taker_action,
            fill.taker_fee_micro_usdc,
        )
    } else {
        return None;
    };
    Some(json!({
        "fill_id": fill.fill_id,
        "ticker": fill.ticker,
        "global_seq": fill.global_seq,
        "contract_seq": fill.contract_seq,
        "order_id": order_id,
        "role": role,
        "side": enum_name(Side::try_from(side).ok()),
        "action": enum_name(Action::try_from(action).ok()),
        "price_ticks": fill.price_ticks,
        "count": fill.count,
        "aggressor_side": enum_name(Side::try_from(fill.aggressor_side).ok()),
        "fee_micro_usdc": fee_micro_usdc,
        "ts": timestamp_json(fill.ts.as_ref()),
    }))
}
fn balance_json(balance: &Balance) -> Value {
    json!({"user_id":balance.user_id,"cash_micro_usdc":balance.cash_micro_usdc,"held_micro_usdc":balance.held_micro_usdc,"total_micro_usdc":balance.total_micro_usdc})
}
fn checked_position_sum(mut values: impl Iterator<Item = i64>) -> Result<i64, Response> {
    values.try_fold(0_i64, |total, value| {
        total.checked_add(value).ok_or_else(|| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "NUMERIC_OVERFLOW",
                "position P&L exceeds supported range",
            )
        })
    })
}
fn checked_position_cost(
    positions: &[sarvex_contracts::sarvex::v1::UserPosition],
) -> Result<i64, Response> {
    let total = positions.iter().try_fold(0_i128, |total, position| {
        let cost =
            i128::from(position.net_qty).abs() * i128::from(position.avg_cost_micro_usdc).abs();
        total.checked_add(cost).ok_or_else(|| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "NUMERIC_OVERFLOW",
                "position cost exceeds supported range",
            )
        })
    })?;
    i64::try_from(total).map_err(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "NUMERIC_OVERFLOW",
            "position cost exceeds supported range",
        )
    })
}
fn user_limits_json(limits: &UserLimits) -> Value {
    json!({
        "user_id": limits.user_id,
        "kyc_tier": limits.kyc_tier,
        "max_order_size_micro_usdc": limits.max_order_size_micro_usdc,
        "daily_loss_limit_micro_usdc": limits.daily_loss_limit_micro_usdc,
        "per_contract_position_limit": limits.per_contract_position_limit,
    })
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
        "book_seq": snapshot.book_seq,
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

fn order_rejection_status(code: &str) -> StatusCode {
    match code {
        "" => StatusCode::OK,
        "UPSTREAM_NOT_FOUND" => StatusCode::BAD_REQUEST,
        "ME_QUEUE_FULL" | "ME_CORE_UNAVAILABLE" => StatusCode::BAD_GATEWAY,
        _ => StatusCode::BAD_REQUEST,
    }
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

    #[test]
    fn account_risk_aggregation_is_integer_and_signed() {
        let positions = vec![
            sarvex_contracts::sarvex::v1::UserPosition {
                net_qty: 3,
                avg_cost_micro_usdc: 12,
                realized_pnl_micro_usdc: 7,
                unrealized_pnl_micro_usdc: -2,
                ..Default::default()
            },
            sarvex_contracts::sarvex::v1::UserPosition {
                net_qty: -2,
                avg_cost_micro_usdc: 20,
                realized_pnl_micro_usdc: -4,
                unrealized_pnl_micro_usdc: 5,
                ..Default::default()
            },
        ];

        assert_eq!(
            checked_position_sum(positions.iter().map(|p| p.realized_pnl_micro_usdc)).ok(),
            Some(3)
        );
        assert_eq!(
            checked_position_sum(positions.iter().map(|p| p.unrealized_pnl_micro_usdc)).ok(),
            Some(3)
        );
        assert_eq!(checked_position_cost(&positions).ok(), Some(76));
    }
}
