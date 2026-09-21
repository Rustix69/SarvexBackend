#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use prost_types::Timestamp;
use sarvex_contracts::sarvex::v1::{
    order_router_client::OrderRouterClient,
    position_server::{Position, PositionServer},
    GetOpenInterestRequest, GetPositionRequest, ListPositionsByContractRequest,
    ListPositionsRequest, ListPositionsResponse, OpenInterest, UserPosition,
};
use sarvex_db::connect;
use sqlx::{postgres::PgPool, QueryBuilder, Row};
use std::{env, net::SocketAddr};
use tonic::{
    transport::{Channel, Endpoint, Server},
    Request, Response, Status,
};

const CONSUMER_NAME: &str = "position-svc.exec.fills";
const STREAM_NAME: &str = "exec.fills.*";

#[derive(Clone)]
struct PositionService {
    pool: PgPool,
}

#[derive(Clone)]
struct HealthState {
    pool: PgPool,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct WireEnvelope {
    event_id: String,
    global_seq: u64,
    payload: WireFill,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct WireFill {
    ticker: String,
    maker_user_id: String,
    taker_user_id: String,
    count: i64,
    maker_side: i32,
    maker_action: i32,
    taker_side: i32,
    taker_action: i32,
}

#[tokio::main]
async fn main() -> Result<()> {
    sarvex_runtime::init_tracing("position-svc");
    let pool = connect().await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("position database readiness check failed")?;
    let order_router =
        OrderRouterClient::new(grpc_channel("ORDER_ROUTER_ADDR", "http://127.0.0.1:50055")?);
    if let Ok(nats_url) = env::var("NATS_URL") {
        tokio::spawn(run_fill_consumer(pool.clone(), order_router, nats_url));
    }
    let grpc_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("GRPC_PORT").unwrap_or_else(|_| "50056".to_owned())
    )
    .parse()?;
    let http_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("HTTP_PORT").unwrap_or_else(|_| "8086".to_owned())
    )
    .parse()?;
    let http = tokio::spawn(run_http(http_addr, HealthState { pool: pool.clone() }));
    let grpc = tokio::spawn(
        Server::builder()
            .add_service(PositionServer::new(PositionService { pool }))
            .serve_with_shutdown(grpc_addr, shutdown_signal()),
    );
    tokio::select! { result = http => result??, result = grpc => result?? }
    Ok(())
}

fn grpc_channel(name: &str, default: &str) -> Result<Channel> {
    Ok(
        Endpoint::from_shared(env::var(name).unwrap_or_else(|_| default.to_owned()))?
            .connect_lazy(),
    )
}

async fn run_http(addr: SocketAddr, state: HealthState) -> Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
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
        Json(serde_json::json!({ "status": "ok", "service": "position-svc" })),
    )
}
async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ready", "service": "position-svc" })),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "status": "not_ready", "error": error.to_string() })),
        ),
    }
}

#[tonic::async_trait]
impl Position for PositionService {
    async fn get_position(
        &self,
        request: Request<GetPositionRequest>,
    ) -> Result<Response<UserPosition>, Status> {
        let request = request.into_inner();
        if request.user_id.trim().is_empty() || request.ticker.trim().is_empty() {
            return Err(Status::invalid_argument("user_id and ticker are required"));
        }
        let row = sqlx::query(POSITION_SELECT)
            .bind(&request.user_id)
            .bind(&request.ticker)
            .fetch_optional(&self.pool)
            .await
            .map_err(internal)?
            .ok_or_else(|| Status::not_found("position not found"))?;
        Ok(Response::new(position_from_row(&row)))
    }

    async fn list_positions(
        &self,
        request: Request<ListPositionsRequest>,
    ) -> Result<Response<ListPositionsResponse>, Status> {
        let request = request.into_inner();
        if request.user_id.trim().is_empty() {
            return Err(Status::invalid_argument("user_id is required"));
        }
        let rows = sqlx::query(POSITION_SELECT_USER)
            .bind(&request.user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        Ok(Response::new(ListPositionsResponse {
            positions: rows.iter().map(position_from_row).collect(),
            next_cursor: String::new(),
        }))
    }

    async fn list_positions_by_contract(
        &self,
        request: Request<ListPositionsByContractRequest>,
    ) -> Result<Response<ListPositionsResponse>, Status> {
        let request = request.into_inner();
        if request.ticker.trim().is_empty() {
            return Err(Status::invalid_argument("ticker is required"));
        }
        let limit = request.limit.clamp(1, 500) as i64;
        let mut query = QueryBuilder::new(POSITION_SELECT);
        query.push(" WHERE ticker = ").push_bind(request.ticker);
        if request.min_global_seq > 0 {
            query
                .push(" AND last_global_seq >= ")
                .push_bind(request.min_global_seq as i64);
        }
        query.push(" ORDER BY user_id LIMIT ").push_bind(limit);
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        Ok(Response::new(ListPositionsResponse {
            positions: rows.iter().map(position_from_row).collect(),
            next_cursor: String::new(),
        }))
    }

    async fn get_open_interest(
        &self,
        request: Request<GetOpenInterestRequest>,
    ) -> Result<Response<OpenInterest>, Status> {
        let ticker = request.into_inner().ticker;
        if ticker.trim().is_empty() {
            return Err(Status::invalid_argument("ticker is required"));
        }
        let row = sqlx::query("SELECT COALESCE(SUM(GREATEST(net_qty,0)),0) AS long_qty, COALESCE(SUM(GREATEST(-net_qty,0)),0) AS short_qty FROM position.positions WHERE ticker=$1").bind(&ticker).fetch_one(&self.pool).await.map_err(internal)?;
        Ok(Response::new(OpenInterest {
            ticker,
            total_open_long: row.get("long_qty"),
            total_open_short: row.get("short_qty"),
        }))
    }
}

const POSITION_SELECT: &str = "SELECT user_id, ticker, net_qty, avg_cost_micro_usdc, realized_pnl_micro_usdc, unrealized_pnl_micro_usdc, updated_at, last_global_seq FROM position.positions";
const POSITION_SELECT_USER: &str = "SELECT user_id, ticker, net_qty, avg_cost_micro_usdc, realized_pnl_micro_usdc, unrealized_pnl_micro_usdc, updated_at, last_global_seq FROM position.positions WHERE user_id=$1 ORDER BY ticker";

fn position_from_row(row: &sqlx::postgres::PgRow) -> UserPosition {
    UserPosition {
        user_id: row.get("user_id"),
        ticker: row.get("ticker"),
        net_qty: row.get("net_qty"),
        avg_cost_micro_usdc: row.get("avg_cost_micro_usdc"),
        realized_pnl_micro_usdc: row.get("realized_pnl_micro_usdc"),
        unrealized_pnl_micro_usdc: row.get("unrealized_pnl_micro_usdc"),
        updated_at: Some(timestamp(row.get("updated_at"))),
        last_global_seq: row.get::<i64, _>("last_global_seq") as u64,
    }
}

async fn run_fill_consumer(
    pool: PgPool,
    order_router: OrderRouterClient<Channel>,
    nats_url: String,
) {
    loop {
        let client = match async_nats::connect(&nats_url).await {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(error = %error, "position NATS connection unavailable");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };
        let mut subscription = match client.subscribe("exec.fills.*".to_owned()).await {
            Ok(subscription) => subscription,
            Err(error) => {
                tracing::warn!(error = %error, "position NATS subscription failed");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };
        tracing::info!(consumer = CONSUMER_NAME, "position fill consumer connected");
        while let Some(message) = subscription.next().await {
            let event: WireEnvelope = match serde_json::from_slice(&message.payload) {
                Ok(event) => event,
                Err(error) => {
                    tracing::warn!(error = %error, "invalid execution event payload");
                    continue;
                }
            };
            if event.global_seq == 0 || event.event_id.trim().is_empty() {
                continue;
            }
            let last = match current_offset(&pool).await {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(error = %error, "position offset read failed");
                    break;
                }
            };
            if event.global_seq > last.saturating_add(1) {
                if let Err(error) = replay_gap(
                    &pool,
                    order_router.clone(),
                    last.saturating_add(1),
                    event.global_seq.saturating_sub(1),
                )
                .await
                {
                    tracing::error!(error = %error, from = last + 1, to = event.global_seq - 1, "position fill replay failed");
                    break;
                }
            }
            if let Err(error) = apply_fill(&pool, &event).await {
                tracing::error!(error = %error, event_id = %event.event_id, "position fill application failed");
                break;
            }
        }
        tracing::warn!("position NATS subscription ended; reconnecting");
    }
}

async fn replay_gap(
    pool: &PgPool,
    mut order_router: OrderRouterClient<Channel>,
    from: u64,
    to: u64,
) -> Result<(), Status> {
    if from > to {
        return Ok(());
    }
    let mut cursor = String::new();
    let mut previous_cursor = None;
    loop {
        let response = order_router
            .list_fills(Request::new(
                sarvex_contracts::sarvex::v1::ListFillsRequest {
                    ticker: String::new(),
                    from_global_seq: from,
                    to_global_seq: to,
                    limit: 500,
                    cursor: cursor.clone(),
                },
            ))
            .await
            .map_err(internal)?
            .into_inner();
        for fill in response.fills {
            apply_fill_record(pool, &fill).await?;
        }
        if response.next_cursor.is_empty() {
            break;
        }
        let next_cursor = response
            .next_cursor
            .parse::<u64>()
            .map_err(|_| Status::internal("order-router returned an invalid fill cursor"))?;
        if previous_cursor.is_some_and(|previous| next_cursor <= previous) {
            return Err(Status::internal(
                "order-router returned a non-advancing fill cursor",
            ));
        }
        previous_cursor = Some(next_cursor);
        cursor = response.next_cursor;
    }
    Ok(())
}

async fn apply_fill_record(
    pool: &PgPool,
    fill: &sarvex_contracts::sarvex::v1::FillRecord,
) -> Result<(), Status> {
    let event = WireEnvelope {
        event_id: fill.fill_id.clone(),
        global_seq: fill.global_seq,
        payload: WireFill {
            ticker: fill.ticker.clone(),
            maker_user_id: fill.maker_user_id.clone(),
            taker_user_id: fill.taker_user_id.clone(),
            count: fill.count,
            maker_side: fill.maker_side,
            maker_action: fill.maker_action,
            taker_side: fill.taker_side,
            taker_action: fill.taker_action,
        },
    };
    apply_fill(pool, &event).await
}

async fn current_offset(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let row =
        sqlx::query("SELECT last_global_seq FROM position.consumer_offsets WHERE consumer_name=$1")
            .bind(CONSUMER_NAME)
            .fetch_optional(pool)
            .await?;
    Ok(row
        .map(|row| row.get::<i64, _>("last_global_seq") as u64)
        .unwrap_or(0))
}

async fn apply_fill(pool: &PgPool, event: &WireEnvelope) -> Result<(), Status> {
    let global_seq = i64::try_from(event.global_seq)
        .map_err(|_| Status::invalid_argument("global sequence exceeds database range"))?;
    let mut tx = pool.begin().await.map_err(internal)?;
    sqlx::query("INSERT INTO position.consumer_offsets (consumer_name, stream_name) VALUES ($1,$2) ON CONFLICT (consumer_name) DO NOTHING").bind(CONSUMER_NAME).bind(STREAM_NAME).execute(&mut *tx).await.map_err(internal)?;
    let inserted = sqlx::query("INSERT INTO position.applied_fills (fill_id, ticker, global_seq) VALUES ($1,$2,$3) ON CONFLICT (fill_id) DO NOTHING RETURNING fill_id").bind(&event.event_id).bind(&event.payload.ticker).bind(global_seq).fetch_optional(&mut *tx).await.map_err(internal)?.is_some();
    if inserted {
        apply_position_delta(
            &mut tx,
            &event.payload.maker_user_id,
            &event.payload.ticker,
            signed_delta(
                event.payload.maker_side,
                event.payload.maker_action,
                event.payload.count,
            ),
            global_seq,
        )
        .await?;
        apply_position_delta(
            &mut tx,
            &event.payload.taker_user_id,
            &event.payload.ticker,
            signed_delta(
                event.payload.taker_side,
                event.payload.taker_action,
                event.payload.count,
            ),
            global_seq,
        )
        .await?;
    }
    sqlx::query("UPDATE position.consumer_offsets SET last_global_seq=GREATEST(last_global_seq,$1), updated_at=now() WHERE consumer_name=$2").bind(global_seq).bind(CONSUMER_NAME).execute(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(())
}

async fn apply_position_delta(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: &str,
    ticker: &str,
    delta: i64,
    global_seq: i64,
) -> Result<(), Status> {
    if user_id.trim().is_empty() || delta == 0 {
        return Ok(());
    }
    sqlx::query("INSERT INTO position.positions (user_id, ticker, net_qty, last_global_seq) VALUES ($1,$2,$3,$4) ON CONFLICT (user_id,ticker) DO UPDATE SET net_qty=position.positions.net_qty+EXCLUDED.net_qty, last_global_seq=GREATEST(position.positions.last_global_seq,EXCLUDED.last_global_seq), updated_at=now()").bind(user_id).bind(ticker).bind(delta).bind(global_seq).execute(&mut **tx).await.map_err(internal)?;
    Ok(())
}

fn signed_delta(side: i32, action: i32, count: i64) -> i64 {
    let side_sign = if side == sarvex_contracts::sarvex::v1::Side::No as i32
        || side == sarvex_contracts::sarvex::v1::Side::Short as i32
    {
        -1
    } else {
        1
    };
    let action_sign = if action == sarvex_contracts::sarvex::v1::Action::Sell as i32 {
        -1
    } else {
        1
    };
    count.saturating_mul(side_sign * action_sign)
}

fn timestamp(value: DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
    }
}
fn internal(error: impl std::fmt::Display) -> Status {
    Status::internal(error.to_string())
}
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_delta_matches_position_side_and_action_rules() {
        assert_eq!(signed_delta(1, 1, 5), 5);
        assert_eq!(signed_delta(1, 2, 5), -5);
        assert_eq!(signed_delta(2, 1, 5), -5);
        assert_eq!(signed_delta(2, 2, 5), 5);
    }

    #[test]
    fn wire_fill_ignores_non_position_fields() {
        let event: WireEnvelope = serde_json::from_str(
            r#"{"event_id":"fill-1","global_seq":7,"payload":{"ticker":"T","maker_user_id":"maker","taker_user_id":"taker","count":3,"maker_side":1,"maker_action":2,"taker_side":1,"taker_action":1,"price_ticks":40}}"#,
        )
        .expect("wire event should decode");
        assert_eq!(event.global_seq, 7);
        assert_eq!(event.payload.ticker, "T");
        assert_eq!(
            signed_delta(
                event.payload.maker_side,
                event.payload.maker_action,
                event.payload.count
            ),
            -3
        );
    }
}
