use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use prost_types::{value::Kind, ListValue, Struct, Timestamp, Value};
use sarvex_contracts::sarvex::v1::{
    ref_data_server::{RefData, RefDataServer},
    Contract, Event, GetContractRequest, GetEventRequest, ListContractsRequest,
    ListContractsResponse, TransitionStateRequest, UpsertContractRequest,
};
use sarvex_db::connect;
use serde_json::Value as JsonValue;
use sqlx::{postgres::PgPool, QueryBuilder, Row};
use std::{env, net::SocketAddr};
use tokio::net::TcpListener;
use tonic::{transport::Server, Request, Response, Status};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

#[derive(Clone)]
struct RefDataService {
    pool: PgPool,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let pool = connect().await?;
    sqlx::query("SELECT 1").execute(&pool).await.context("refdata database readiness check failed")?;

    let grpc_port = env::var("GRPC_PORT").unwrap_or_else(|_| "50051".to_owned());
    let http_port = env::var("HTTP_PORT").unwrap_or_else(|_| "8080".to_owned());
    let grpc_addr: SocketAddr = format!("0.0.0.0:{grpc_port}").parse()?;
    let http_addr: SocketAddr = format!("0.0.0.0:{http_port}").parse()?;
    let grpc_service = RefDataService { pool: pool.clone() };
    let http_state = AppState { pool };

    let http = tokio::spawn(async move { run_http(http_addr, http_state).await });
    let grpc = tokio::spawn(async move {
        Server::builder()
            .add_service(RefDataServer::new(grpc_service))
            .serve_with_shutdown(grpc_addr, shutdown_signal())
            .await
            .context("refdata gRPC server failed")
    });

    tokio::select! {
        result = http => result??,
        result = grpc => result??,
    }
    Ok(())
}

async fn run_http(addr: SocketAddr, state: AppState) -> Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;
    Ok(())
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok", "service": "refdata-svc" })))
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ready", "service": "refdata-svc" }))),
        Err(error) => (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "status": "not_ready", "error": error.to_string() }))),
    }
}

#[tonic::async_trait]
impl RefData for RefDataService {
    async fn get_contract(&self, request: Request<GetContractRequest>) -> Result<Response<Contract>, Status> {
        let ticker = request.into_inner().ticker;
        let row = sqlx::query(CONTRACT_SELECT).bind(&ticker).fetch_optional(&self.pool).await.map_err(internal)?;
        row.map(|row| Ok(Response::new(contract_from_row(&row))))
            .unwrap_or_else(|| Err(Status::not_found("contract not found")))
    }

    async fn list_contracts(&self, request: Request<ListContractsRequest>) -> Result<Response<ListContractsResponse>, Status> {
        let request = request.into_inner();
        let limit = request.limit.clamp(1, 200) as i64;
        let mut query = QueryBuilder::new(CONTRACT_SELECT_BASE);
        let mut has_where = false;
        if request.state != 0 {
            let state = contract_state_name(request.state).ok_or_else(|| Status::invalid_argument("invalid contract state"))?;
            query.push(" WHERE state::text = ").push_bind(state);
            has_where = true;
        }
        if !request.series_ticker.trim().is_empty() {
            query.push(if has_where { " AND " } else { " WHERE " });
            query.push("series_ticker = ").push_bind(request.series_ticker.trim().to_owned());
            has_where = true;
        }
        if !request.cursor.trim().is_empty() {
            query.push(if has_where { " AND " } else { " WHERE " });
            query.push("ticker > ").push_bind(request.cursor.trim().to_owned());
        }
        query.push(" ORDER BY ticker LIMIT ").push_bind(limit + 1);
        let rows = query.build().fetch_all(&self.pool).await.map_err(internal)?;
        let has_next = rows.len() > limit as usize;
        let rows = rows.into_iter().take(limit as usize).collect::<Vec<_>>();
        let next_cursor = if has_next { rows.last().map(|row| row.get::<String, _>("ticker")).unwrap_or_default() } else { String::new() };
        Ok(Response::new(ListContractsResponse {
            contracts: rows.iter().map(contract_from_row).collect(),
            next_cursor,
        }))
    }

    async fn transition_state(&self, request: Request<TransitionStateRequest>) -> Result<Response<prost_types::Empty>, Status> {
        let request = request.into_inner();
        let new_state = contract_state_name(request.new_state).ok_or_else(|| Status::invalid_argument("invalid contract state"))?;
        let mut tx = self.pool.begin().await.map_err(internal)?;
        let old = sqlx::query("SELECT state::text FROM refdata.contracts WHERE ticker = $1 FOR UPDATE")
            .bind(&request.ticker).fetch_optional(&mut *tx).await.map_err(internal)?;
        let old = old.map(|row| row.get::<String, _>(0)).ok_or_else(|| Status::not_found("contract not found"))?;
        sqlx::query("UPDATE refdata.contracts SET state = $1::refdata.contract_state, updated_at = now() WHERE ticker = $2")
            .bind(new_state).bind(&request.ticker).execute(&mut *tx).await.map_err(internal)?;
        sqlx::query("INSERT INTO refdata.contract_state_history (ticker, old_state, new_state, reason, changed_by) VALUES ($1, $2::refdata.contract_state, $3::refdata.contract_state, $4, 'refdata-svc')")
            .bind(&request.ticker).bind(old).bind(new_state).bind(request.reason).execute(&mut *tx).await.map_err(internal)?;
        tx.commit().await.map_err(internal)?;
        Ok(Response::new(prost_types::Empty {}))
    }

    async fn upsert_contract(&self, _request: Request<UpsertContractRequest>) -> Result<Response<Contract>, Status> {
        Err(Status::unimplemented("contract upsert is not enabled in Phase 01"))
    }

    async fn get_event(&self, request: Request<GetEventRequest>) -> Result<Response<Event>, Status> {
        let row = sqlx::query("SELECT event_ticker, series_ticker, title, description, expected_resolution_at FROM refdata.events WHERE event_ticker = $1")
            .bind(request.into_inner().event_ticker).fetch_optional(&self.pool).await.map_err(internal)?
            .ok_or_else(|| Status::not_found("event not found"))?;
        Ok(Response::new(Event {
            event_ticker: row.get("event_ticker"),
            series_ticker: row.get("series_ticker"),
            title: row.get("title"),
            description: row.get("description"),
            expected_resolution_at: timestamp(row.get("expected_resolution_at")),
        }))
    }
}

const CONTRACT_SELECT_BASE: &str = "SELECT ticker, event_ticker, series_ticker, kind::text AS kind, question, underlying, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, multiplier_micro_usdc, max_order_size, position_limit_per_user, state::text AS state, listed_at, open_at, close_at, expected_resolution_at, settlement_source, oracle_policy, settlement_rule, close_global_seq FROM refdata.contracts";
const CONTRACT_SELECT: &str = "SELECT ticker, event_ticker, series_ticker, kind::text AS kind, question, underlying, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, multiplier_micro_usdc, max_order_size, position_limit_per_user, state::text AS state, listed_at, open_at, close_at, expected_resolution_at, settlement_source, oracle_policy, settlement_rule, close_global_seq FROM refdata.contracts WHERE ticker = $1";

fn contract_from_row(row: &sqlx::postgres::PgRow) -> Contract {
    Contract {
        ticker: row.get("ticker"),
        event_ticker: row.get("event_ticker"),
        series_ticker: row.get("series_ticker"),
        kind: match row.get::<String, _>("kind").as_str() { "SCALAR" => 2, _ => 1 },
        question: row.get::<Option<String>, _>("question").unwrap_or_default(),
        underlying: row.get::<Option<String>, _>("underlying").unwrap_or_default(),
        tick_size: row.get("tick_size"),
        min_price_ticks: row.get("min_price_ticks"),
        max_price_ticks: row.get("max_price_ticks"),
        lower_bound_ticks: row.get::<Option<i64>, _>("lower_bound_ticks").unwrap_or_default(),
        upper_bound_ticks: row.get::<Option<i64>, _>("upper_bound_ticks").unwrap_or_default(),
        multiplier_micro_usdc: row.get::<Option<i64>, _>("multiplier_micro_usdc").unwrap_or_default(),
        max_order_size: row.get("max_order_size"),
        position_limit_per_user: row.get("position_limit_per_user"),
        state: contract_state_value(&row.get::<String, _>("state")),
        listed_at: optional_timestamp(row.get("listed_at")),
        open_at: optional_timestamp(row.get("open_at")),
        close_at: optional_timestamp(row.get("close_at")),
        expected_resolution_at: timestamp(row.get("expected_resolution_at")),
        settlement_source: row.get::<Option<String>, _>("settlement_source").unwrap_or_default(),
        oracle_policy: row.get::<Option<String>, _>("oracle_policy").unwrap_or_default(),
        settlement_rule: json_to_struct(row.get::<serde_json::Value, _>("settlement_rule")),
        close_global_seq: row.get::<Option<i64>, _>("close_global_seq").unwrap_or_default() as u64,
    }
}

fn contract_state_name(value: i32) -> Option<&'static str> {
    match value { 1 => Some("DRAFT"), 2 => Some("LISTED"), 3 => Some("OPEN"), 4 => Some("CLOSED"), 5 => Some("RESOLVING"), 6 => Some("SETTLED"), 7 => Some("CANCELLED"), 8 => Some("HALTED"), _ => None }
}

fn contract_state_value(value: &str) -> i32 {
    match value { "DRAFT" => 1, "LISTED" => 2, "OPEN" => 3, "CLOSED" => 4, "RESOLVING" => 5, "SETTLED" => 6, "CANCELLED" => 7, "HALTED" => 8, _ => 0 }
}

fn timestamp(value: DateTime<Utc>) -> Option<Timestamp> {
    Some(Timestamp { seconds: value.timestamp(), nanos: value.timestamp_subsec_nanos() as i32 })
}

fn optional_timestamp(value: Option<DateTime<Utc>>) -> Option<Timestamp> { value.map(|v| Timestamp { seconds: v.timestamp(), nanos: v.timestamp_subsec_nanos() as i32 }) }

fn json_to_struct(value: JsonValue) -> Option<Struct> {
    match value { JsonValue::Object(map) => Some(Struct { fields: map.into_iter().map(|(key, value)| (key, json_to_value(value))).collect() }), _ => None }
}

fn json_to_value(value: JsonValue) -> Value {
    let kind = match value {
        JsonValue::Null => Kind::NullValue(0),
        JsonValue::Bool(value) => Kind::BoolValue(value),
        JsonValue::Number(value) => Kind::NumberValue(value.as_f64().unwrap_or_default()),
        JsonValue::String(value) => Kind::StringValue(value),
        JsonValue::Array(values) => Kind::ListValue(ListValue { values: values.into_iter().map(json_to_value).collect() }),
        JsonValue::Object(values) => Kind::StructValue(Struct { fields: values.into_iter().map(|(key, value)| (key, json_to_value(value))).collect() }),
    };
    Value { kind: Some(kind) }
}

fn internal(error: impl std::fmt::Display) -> Status { Status::internal(error.to_string()) }

fn init_tracing() { let _ = tracing_subscriber::fmt().with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "info".to_owned())).with_target(false).try_init(); }

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
