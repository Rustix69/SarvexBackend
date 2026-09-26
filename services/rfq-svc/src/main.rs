#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use prost_types::Timestamp;
use sarvex_contracts::sarvex::v1::{
    ref_data_client::RefDataClient,
    rfq_service_server::{RfqService, RfqServiceServer},
    AcceptQuoteRequest, CancelQuoteRequest, CancelRfqRequest, CreateRfqRequest, GetRfqRequest,
    ListQuotesRequest, ListQuotesResponse, Rfq, RfqQuote, RfqQuoteStatus, RfqStatus,
    SubmitQuoteRequest,
};
use sarvex_db::connect;
use sqlx::{postgres::PgPool, QueryBuilder, Row};
use std::{env, net::SocketAddr, time::Duration};
use tonic::{
    transport::{Channel, Endpoint, Server},
    Request, Response, Status,
};
use uuid::Uuid;

#[derive(Clone)]
struct RfqServiceImpl {
    pool: PgPool,
    refdata: RefDataClient<Channel>,
}

#[derive(Clone)]
struct HealthState {
    pool: PgPool,
}

const RFQ_SELECT: &str = "SELECT rfq_id, client_rfq_id, creator_user_id, ticker, side, action, requested_count, expires_at, status, accepted_quote_id, created_at, updated_at FROM rfq.requests";
const QUOTE_SELECT: &str = "SELECT quote_id, rfq_id, maker_user_id, bid_price_ticks, offer_price_ticks, available_count, expires_at, status, created_at, updated_at FROM rfq.quotes";

#[tokio::main]
async fn main() -> Result<()> {
    sarvex_runtime::init_tracing("rfq-svc");
    let pool = connect().await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("rfq database readiness check failed")?;
    let refdata = RefDataClient::new(grpc_channel("REFDATA_ADDR", "http://127.0.0.1:50051")?);
    tokio::spawn(expire_loop(pool.clone()));
    let grpc_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("GRPC_PORT").unwrap_or_else(|_| "50059".to_owned())
    )
    .parse()?;
    let http_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("HTTP_PORT").unwrap_or_else(|_| "8091".to_owned())
    )
    .parse()?;
    let http = tokio::spawn(run_http(http_addr, HealthState { pool: pool.clone() }));
    let grpc = tokio::spawn(
        Server::builder()
            .add_service(RfqServiceServer::new(RfqServiceImpl { pool, refdata }))
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
        Json(serde_json::json!({"status":"ok","service":"rfq-svc"})),
    )
}

async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ready","service":"rfq-svc"})),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                serde_json::json!({"status":"not_ready","service":"rfq-svc","error":error.to_string()}),
            ),
        ),
    }
}

#[tonic::async_trait]
impl RfqService for RfqServiceImpl {
    async fn create_rfq(
        &self,
        request: Request<CreateRfqRequest>,
    ) -> Result<Response<Rfq>, Status> {
        let request = request.into_inner();
        if request.creator_user_id.trim().is_empty()
            || request.client_rfq_id.trim().is_empty()
            || request.ticker.trim().is_empty()
            || request.requested_count <= 0
            || request.side == 0
            || request.action == 0
        {
            return Err(Status::invalid_argument("creator, client_rfq_id, ticker, side, action and positive requested_count are required"));
        }
        let expires_at = timestamp_to_datetime(request.expires_at)?;
        if expires_at <= Utc::now() {
            return Err(Status::invalid_argument("expires_at must be in the future"));
        }
        let contract = self
            .refdata
            .clone()
            .get_contract(sarvex_contracts::sarvex::v1::GetContractRequest {
                ticker: request.ticker.clone(),
            })
            .await
            .map_err(internal)?
            .into_inner();
        if contract.state != sarvex_contracts::sarvex::v1::ContractState::Open as i32 {
            return Err(Status::failed_precondition("contract is not open"));
        }
        sqlx::query("INSERT INTO rfq.requests (rfq_id, client_rfq_id, creator_user_id, ticker, side, action, requested_count, expires_at, status) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'OPEN') ON CONFLICT (creator_user_id, client_rfq_id) DO NOTHING")
            .bind(format!("rfq_{}", Uuid::new_v4()))
            .bind(&request.client_rfq_id)
            .bind(&request.creator_user_id)
            .bind(&request.ticker)
            .bind(request.side)
            .bind(request.action)
            .bind(request.requested_count)
            .bind(expires_at)
            .execute(&self.pool)
            .await
            .map_err(internal)?;
        let row = sqlx::query(&format!(
            "{RFQ_SELECT} WHERE creator_user_id=$1 AND client_rfq_id=$2"
        ))
        .bind(&request.creator_user_id)
        .bind(&request.client_rfq_id)
        .fetch_one(&self.pool)
        .await
        .map_err(internal)?;
        Ok(Response::new(rfq_from_row(&row)))
    }

    async fn get_rfq(&self, request: Request<GetRfqRequest>) -> Result<Response<Rfq>, Status> {
        let request = request.into_inner();
        let row = sqlx::query(&format!(
            "{RFQ_SELECT} WHERE rfq_id=$1 AND creator_user_id=$2"
        ))
        .bind(&request.rfq_id)
        .bind(&request.user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(internal)?
        .ok_or_else(|| Status::not_found("rfq not found"))?;
        Ok(Response::new(rfq_from_row(&row)))
    }

    async fn list_quotes(
        &self,
        request: Request<ListQuotesRequest>,
    ) -> Result<Response<ListQuotesResponse>, Status> {
        let request = request.into_inner();
        if request.user_id.trim().is_empty() || request.rfq_id.trim().is_empty() {
            return Err(Status::invalid_argument("user_id and rfq_id are required"));
        }
        let limit = i64::from(if request.limit <= 0 {
            100
        } else {
            request.limit.clamp(1, 500)
        });
        let mut query = QueryBuilder::new(format!(
            "{QUOTE_SELECT} q JOIN rfq.requests r ON r.rfq_id=q.rfq_id WHERE q.rfq_id = "
        ));
        query.push_bind(&request.rfq_id);
        query
            .push(" AND (r.creator_user_id = ")
            .push_bind(&request.user_id)
            .push(" OR q.maker_user_id = ")
            .push_bind(&request.user_id)
            .push(")");
        if !request.cursor.trim().is_empty() {
            query.push(" AND q.quote_id > ").push_bind(&request.cursor);
        }
        query.push(" ORDER BY q.quote_id LIMIT ").push_bind(limit);
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        let next_cursor = if rows.len() == limit as usize {
            rows.last()
                .map(|row| row.get::<String, _>("quote_id"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        Ok(Response::new(ListQuotesResponse {
            quotes: rows.iter().map(quote_from_row).collect(),
            next_cursor,
        }))
    }

    async fn cancel_rfq(
        &self,
        request: Request<CancelRfqRequest>,
    ) -> Result<Response<Rfq>, Status> {
        let request = request.into_inner();
        let mut tx = self.pool.begin().await.map_err(internal)?;
        let row = sqlx::query(&format!(
            "{RFQ_SELECT} WHERE rfq_id=$1 AND creator_user_id=$2 FOR UPDATE"
        ))
        .bind(&request.rfq_id)
        .bind(&request.creator_user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?
        .ok_or_else(|| Status::not_found("rfq not found"))?;
        let status: String = row.get("status");
        if status != "OPEN" {
            return Err(Status::failed_precondition("rfq is not open"));
        }
        sqlx::query("UPDATE rfq.requests SET status='CANCELLED', updated_at=now() WHERE rfq_id=$1")
            .bind(&request.rfq_id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        sqlx::query("UPDATE rfq.quotes SET status='CANCELLED', updated_at=now() WHERE rfq_id=$1 AND status='PENDING'")
            .bind(&request.rfq_id).execute(&mut *tx).await.map_err(internal)?;
        tx.commit().await.map_err(internal)?;
        let row = sqlx::query(&format!("{RFQ_SELECT} WHERE rfq_id=$1"))
            .bind(&request.rfq_id)
            .fetch_one(&self.pool)
            .await
            .map_err(internal)?;
        Ok(Response::new(rfq_from_row(&row)))
    }

    async fn submit_quote(
        &self,
        request: Request<SubmitQuoteRequest>,
    ) -> Result<Response<RfqQuote>, Status> {
        let request = request.into_inner();
        if request.maker_user_id.trim().is_empty()
            || request.quote_id.trim().is_empty()
            || request.rfq_id.trim().is_empty()
            || request.available_count <= 0
            || request.bid_price_ticks <= 0
            || request.offer_price_ticks <= 0
            || request.bid_price_ticks > request.offer_price_ticks
        {
            return Err(Status::invalid_argument(
                "maker, quote_id, rfq_id, positive prices/count and bid <= offer are required",
            ));
        }
        let quote_expires_at = timestamp_to_datetime(request.expires_at)?;
        if quote_expires_at <= Utc::now() {
            return Err(Status::invalid_argument(
                "quote expires_at must be in the future",
            ));
        }
        let rfq = sqlx::query(
            "SELECT ticker, status, expires_at, creator_user_id FROM rfq.requests WHERE rfq_id=$1",
        )
        .bind(&request.rfq_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(internal)?
        .ok_or_else(|| Status::not_found("rfq not found"))?;
        let status: String = rfq.get("status");
        let rfq_expires: DateTime<Utc> = rfq.get("expires_at");
        if status != "OPEN" || rfq_expires <= Utc::now() {
            return Err(Status::failed_precondition("rfq is not open"));
        }
        if rfq.get::<String, _>("creator_user_id") == request.maker_user_id {
            return Err(Status::permission_denied(
                "rfq creator cannot quote its own request",
            ));
        }
        sqlx::query("INSERT INTO rfq.quotes (quote_id, rfq_id, maker_user_id, bid_price_ticks, offer_price_ticks, available_count, expires_at, status) VALUES ($1,$2,$3,$4,$5,$6,$7,'PENDING')")
            .bind(&request.quote_id).bind(&request.rfq_id).bind(&request.maker_user_id).bind(request.bid_price_ticks).bind(request.offer_price_ticks).bind(request.available_count).bind(quote_expires_at).execute(&self.pool).await.map_err(|error| if is_unique_violation(&error) { Status::already_exists("quote_id already exists") } else { internal(error) })?;
        let row = sqlx::query(&format!("{QUOTE_SELECT} WHERE quote_id=$1"))
            .bind(&request.quote_id)
            .fetch_one(&self.pool)
            .await
            .map_err(internal)?;
        Ok(Response::new(quote_from_row(&row)))
    }

    async fn cancel_quote(
        &self,
        request: Request<CancelQuoteRequest>,
    ) -> Result<Response<RfqQuote>, Status> {
        let request = request.into_inner();
        let result = sqlx::query("UPDATE rfq.quotes SET status='CANCELLED', updated_at=now() WHERE quote_id=$1 AND maker_user_id=$2 AND status='PENDING'")
            .bind(&request.quote_id).bind(&request.maker_user_id).execute(&self.pool).await.map_err(internal)?;
        if result.rows_affected() == 0 {
            return Err(Status::failed_precondition(
                "quote is not pending or not owned by maker",
            ));
        }
        let row = sqlx::query(&format!("{QUOTE_SELECT} WHERE quote_id=$1"))
            .bind(&request.quote_id)
            .fetch_one(&self.pool)
            .await
            .map_err(internal)?;
        Ok(Response::new(quote_from_row(&row)))
    }

    async fn accept_quote(
        &self,
        request: Request<AcceptQuoteRequest>,
    ) -> Result<Response<Rfq>, Status> {
        let request = request.into_inner();
        let mut tx = self.pool.begin().await.map_err(internal)?;
        let rfq = sqlx::query(&format!(
            "{RFQ_SELECT} WHERE rfq_id=$1 AND creator_user_id=$2 FOR UPDATE"
        ))
        .bind(&request.rfq_id)
        .bind(&request.creator_user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?
        .ok_or_else(|| Status::not_found("rfq not found"))?;
        let status: String = rfq.get("status");
        let expires_at: DateTime<Utc> = rfq.get("expires_at");
        if expires_at <= Utc::now() {
            return Err(Status::failed_precondition("rfq has expired"));
        }
        if status != "OPEN" {
            return Err(Status::failed_precondition("rfq is not open"));
        }
        let quote = sqlx::query("SELECT status, available_count FROM rfq.quotes WHERE quote_id=$1 AND rfq_id=$2 FOR UPDATE")
            .bind(&request.quote_id).bind(&request.rfq_id).fetch_optional(&mut *tx).await.map_err(internal)?.ok_or_else(|| Status::not_found("quote not found"))?;
        if quote.get::<String, _>("status") != "PENDING" {
            return Err(Status::failed_precondition("quote is not pending"));
        }
        if quote.get::<i64, _>("available_count") < rfq.get::<i64, _>("requested_count") {
            return Err(Status::failed_precondition(
                "quote quantity is insufficient",
            ));
        }
        sqlx::query("UPDATE rfq.requests SET status='ACCEPTED_PENDING_EXECUTION', accepted_quote_id=$1, updated_at=now() WHERE rfq_id=$2")
            .bind(&request.quote_id).bind(&request.rfq_id).execute(&mut *tx).await.map_err(internal)?;
        sqlx::query("UPDATE rfq.quotes SET status=CASE WHEN quote_id=$1 THEN 'ACCEPTED' ELSE 'CANCELLED' END, updated_at=now() WHERE rfq_id=$2 AND status='PENDING'")
            .bind(&request.quote_id).bind(&request.rfq_id).execute(&mut *tx).await.map_err(internal)?;
        tx.commit().await.map_err(internal)?;
        let row = sqlx::query(&format!("{RFQ_SELECT} WHERE rfq_id=$1"))
            .bind(&request.rfq_id)
            .fetch_one(&self.pool)
            .await
            .map_err(internal)?;
        Ok(Response::new(rfq_from_row(&row)))
    }
}

async fn expire_loop(pool: PgPool) {
    loop {
        let _ = sqlx::query("UPDATE rfq.requests SET status='EXPIRED', updated_at=now() WHERE status='OPEN' AND expires_at <= now()").execute(&pool).await;
        let _ = sqlx::query("UPDATE rfq.quotes SET status='EXPIRED', updated_at=now() WHERE status='PENDING' AND expires_at <= now()").execute(&pool).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn rfq_from_row(row: &sqlx::postgres::PgRow) -> Rfq {
    Rfq {
        rfq_id: row.get("rfq_id"),
        client_rfq_id: row.get("client_rfq_id"),
        creator_user_id: row.get("creator_user_id"),
        ticker: row.get("ticker"),
        side: row.get("side"),
        action: row.get("action"),
        requested_count: row.get("requested_count"),
        expires_at: Some(timestamp(row.get("expires_at"))),
        status: rfq_status(row.get::<String, _>("status").as_str()),
        accepted_quote_id: row
            .get::<Option<String>, _>("accepted_quote_id")
            .unwrap_or_default(),
        created_at: Some(timestamp(row.get("created_at"))),
        updated_at: Some(timestamp(row.get("updated_at"))),
    }
}

fn quote_from_row(row: &sqlx::postgres::PgRow) -> RfqQuote {
    RfqQuote {
        quote_id: row.get("quote_id"),
        rfq_id: row.get("rfq_id"),
        maker_user_id: row.get("maker_user_id"),
        bid_price_ticks: row.get("bid_price_ticks"),
        offer_price_ticks: row.get("offer_price_ticks"),
        available_count: row.get("available_count"),
        expires_at: Some(timestamp(row.get("expires_at"))),
        status: quote_status(row.get::<String, _>("status").as_str()),
        created_at: Some(timestamp(row.get("created_at"))),
        updated_at: Some(timestamp(row.get("updated_at"))),
    }
}

fn rfq_status(value: &str) -> i32 {
    match value {
        "OPEN" => RfqStatus::Open as i32,
        "ACCEPTED_PENDING_EXECUTION" => RfqStatus::AcceptedPendingExecution as i32,
        "EXECUTED" => RfqStatus::Executed as i32,
        "CANCELLED" => RfqStatus::Cancelled as i32,
        "EXPIRED" => RfqStatus::Expired as i32,
        "REJECTED" => RfqStatus::Rejected as i32,
        _ => RfqStatus::Unspecified as i32,
    }
}
fn quote_status(value: &str) -> i32 {
    match value {
        "PENDING" => RfqQuoteStatus::Pending as i32,
        "ACCEPTED" => RfqQuoteStatus::Accepted as i32,
        "REJECTED" => RfqQuoteStatus::Rejected as i32,
        "CANCELLED" => RfqQuoteStatus::Cancelled as i32,
        "EXPIRED" => RfqQuoteStatus::Expired as i32,
        _ => RfqQuoteStatus::Unspecified as i32,
    }
}
fn timestamp(value: DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
    }
}
fn timestamp_to_datetime(value: Option<Timestamp>) -> Result<DateTime<Utc>, Status> {
    let value = value.ok_or_else(|| Status::invalid_argument("expires_at is required"))?;
    DateTime::<Utc>::from_timestamp(value.seconds, value.nanos as u32)
        .ok_or_else(|| Status::invalid_argument("invalid expires_at"))
}
fn internal<E: std::fmt::Display>(error: E) -> Status {
    Status::internal(error.to_string())
}
fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.code().as_deref() == Some("23505"))
}
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
