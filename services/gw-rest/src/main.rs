use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use sarvex_contracts::sarvex::v1::{ref_data_client::RefDataClient, ContractState, GetContractRequest, ListContractsRequest};
use serde::{Deserialize, Serialize};
use std::{env, net::SocketAddr};
use tonic::transport::{Channel, Endpoint};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[derive(Clone)]
struct AppState {
    refdata: RefDataClient<Channel>,
}

#[derive(Debug, Deserialize)]
struct MarketQuery {
    state: Option<String>,
    series_ticker: Option<String>,
    limit: Option<i32>,
    cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct ContractResponse {
    ticker: String,
    event_ticker: String,
    series_ticker: String,
    kind: i32,
    question: String,
    underlying: String,
    tick_size: i64,
    min_price_ticks: i64,
    max_price_ticks: i64,
    lower_bound_ticks: i64,
    upper_bound_ticks: i64,
    multiplier_micro_usdc: i64,
    max_order_size: i64,
    position_limit_per_user: i64,
    state: i32,
    listed_at: Option<String>,
    open_at: Option<String>,
    close_at: Option<String>,
    expected_resolution_at: Option<String>,
    settlement_source: String,
    oracle_policy: String,
    settlement_rule: Option<prost_types::Struct>,
    close_global_seq: u64,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    contracts: Vec<ContractResponse>,
    next_cursor: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt().with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "info".to_owned())).with_target(false).try_init();
    let address = env::var("REFDATA_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50061".to_owned());
    let channel = Endpoint::from_shared(address)?.connect_lazy();
    let state = AppState { refdata: RefDataClient::new(channel) };
    let port = env::var("HTTP_PORT").unwrap_or_else(|_| "18080".to_owned());
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    let cors = CorsLayer::permissive();
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/markets", get(list_markets))
        .route("/v1/markets/{ticker}", get(get_market))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> impl IntoResponse { (StatusCode::OK, Json(serde_json::json!({ "status": "ok", "service": "gw-rest" }))) }
async fn readyz() -> impl IntoResponse { (StatusCode::OK, Json(serde_json::json!({ "status": "ready", "service": "gw-rest" }))) }

async fn list_markets(State(state): State<AppState>, Query(query): Query<MarketQuery>) -> impl IntoResponse {
    let mut client = state.refdata.clone();
    let request = ListContractsRequest {
        state: query.state.as_deref().and_then(state_value).unwrap_or(0),
        series_ticker: query.series_ticker.unwrap_or_default(),
        limit: query.limit.unwrap_or(50),
        cursor: query.cursor.unwrap_or_default(),
    };
    match client.list_contracts(request).await {
        Ok(response) => Json(ListResponse { contracts: response.get_ref().contracts.iter().map(contract_response).collect(), next_cursor: response.get_ref().next_cursor.clone() }).into_response(),
        Err(error) => grpc_error(error.code().to_string(), error.message()),
    }
}

async fn get_market(State(state): State<AppState>, Path(ticker): Path<String>) -> impl IntoResponse {
    let mut client = state.refdata.clone();
    match client.get_contract(GetContractRequest { ticker }).await {
        Ok(response) => Json(contract_response(response.get_ref())).into_response(),
        Err(error) => grpc_error(error.code().to_string(), error.message()),
    }
}

fn state_value(value: &str) -> Option<i32> {
    match value.to_ascii_uppercase().as_str() {
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

fn contract_response(contract: &sarvex_contracts::sarvex::v1::Contract) -> ContractResponse {
    ContractResponse {
        ticker: contract.ticker.clone(), event_ticker: contract.event_ticker.clone(), series_ticker: contract.series_ticker.clone(), kind: contract.kind,
        question: contract.question.clone(), underlying: contract.underlying.clone(), tick_size: contract.tick_size, min_price_ticks: contract.min_price_ticks, max_price_ticks: contract.max_price_ticks,
        lower_bound_ticks: contract.lower_bound_ticks, upper_bound_ticks: contract.upper_bound_ticks, multiplier_micro_usdc: contract.multiplier_micro_usdc, max_order_size: contract.max_order_size,
        position_limit_per_user: contract.position_limit_per_user, state: contract.state, listed_at: format_timestamp(contract.listed_at.as_ref()), open_at: format_timestamp(contract.open_at.as_ref()), close_at: format_timestamp(contract.close_at.as_ref()), expected_resolution_at: format_timestamp(contract.expected_resolution_at.as_ref()),
        settlement_source: contract.settlement_source.clone(), oracle_policy: contract.oracle_policy.clone(), settlement_rule: contract.settlement_rule.clone(), close_global_seq: contract.close_global_seq,
    }
}

fn format_timestamp(value: Option<&prost_types::Timestamp>) -> Option<String> {
    value.and_then(|value| DateTime::<Utc>::from_timestamp(value.seconds, value.nanos as u32)).map(|value| value.to_rfc3339())
}

fn grpc_error(code: String, message: &str) -> axum::response::Response {
    let status = if code == "not_found" { StatusCode::NOT_FOUND } else { StatusCode::BAD_GATEWAY };
    (status, Json(serde_json::json!({ "error": { "code": code, "message": message } }))).into_response()
}
