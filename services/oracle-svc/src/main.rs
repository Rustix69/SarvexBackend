#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use chrono::{DateTime, Duration, Utc};
use prost_types::Timestamp;
use sarvex_contracts::sarvex::v1::{
    oracle_server::{Oracle, OracleServer},
    AdminForceResolutionRequest, Attestation, FinalizeResolutionRequest, GetResolutionRequest,
    ProposeResolutionRequest, Resolution, ResolutionStatus,
};
use sarvex_db::connect;
use sarvex_events::EventPublisher;
use sqlx::{postgres::PgPool, Row};
use std::{env, net::SocketAddr};
use tonic::{transport::Server, Request, Response, Status};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[derive(Clone)]
struct OracleService {
    pool: PgPool,
    publisher: Option<EventPublisher>,
    quorum: i32,
    challenge: i64,
}
#[derive(Clone)]
struct HealthState {
    pool: PgPool,
}

#[tokio::main]
async fn main() -> Result<()> {
    sarvex_runtime::init_tracing("oracle-svc");
    let pool = connect().await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("oracle database readiness check failed")?;
    let publisher = match env::var("NATS_URL") {
        Ok(url) => Some(EventPublisher::connect(&url).await?),
        Err(_) => None,
    };
    let service = OracleService {
        pool: pool.clone(),
        publisher,
        quorum: env::var("ORACLE_REQUIRED_QUORUM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
        challenge: env::var("ORACLE_CHALLENGE_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    };
    let grpc_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("GRPC_PORT").unwrap_or_else(|_| "50057".into())
    )
    .parse()?;
    let http_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("HTTP_PORT").unwrap_or_else(|_| "8088".into())
    )
    .parse()?;
    let http = tokio::spawn(run_http(http_addr, HealthState { pool }));
    let grpc = tokio::spawn(
        Server::builder()
            .add_service(OracleServer::new(service))
            .serve_with_shutdown(grpc_addr, shutdown_signal()),
    );
    tokio::select! { result = http => result??, result = grpc => result?? }
    Ok(())
}

async fn run_http(addr: SocketAddr, state: HealthState) -> Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}
async fn healthz() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({"status":"ok","service":"oracle-svc"})),
    )
}
async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ready","service":"oracle-svc"})),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status":"not_ready","error":error.to_string()})),
        ),
    }
}

#[tonic::async_trait]
impl Oracle for OracleService {
    async fn propose_resolution(
        &self,
        request: Request<ProposeResolutionRequest>,
    ) -> Result<Response<Resolution>, Status> {
        let input = request.into_inner();
        validate_value(&input.event_ticker, &input.categorical_value)?;
        require_text(&input.attestor_id, "attestor_id")?;
        require_text(&input.source, "source")?;
        let mut tx = self.pool.begin().await.map_err(internal)?;
        let numeric = if input.categorical_value.is_empty() {
            Some(input.numeric_value)
        } else {
            None
        };
        sqlx::query("INSERT INTO oracle.attestations (event_ticker,attestor_id,source,numeric_value,categorical_value,signature,observed_at) VALUES ($1,$2,$3,$4,NULLIF($5,''),$6,now()) ON CONFLICT (event_ticker,attestor_id,source) DO UPDATE SET numeric_value=EXCLUDED.numeric_value,categorical_value=EXCLUDED.categorical_value,signature=EXCLUDED.signature,observed_at=EXCLUDED.observed_at,received_at=now()")
            .bind(&input.event_ticker).bind(&input.attestor_id).bind(&input.source).bind(numeric).bind(&input.categorical_value).bind(&input.signature).execute(&mut *tx).await.map_err(internal)?;
        let challenge_end =
            (self.challenge > 0).then(|| Utc::now() + Duration::seconds(self.challenge));
        sqlx::query("INSERT INTO oracle.resolutions (event_ticker,status,numeric_value,categorical_value,proposed_at,challenge_window_ends_at,attestor_count,required_quorum) VALUES ($1,'PROPOSED'::oracle.resolution_status,$2,NULLIF($3,''),now(),$4,(SELECT COUNT(DISTINCT attestor_id) FROM oracle.attestations WHERE event_ticker=$1),$5) ON CONFLICT (event_ticker) DO UPDATE SET status='PROPOSED'::oracle.resolution_status,numeric_value=EXCLUDED.numeric_value,categorical_value=EXCLUDED.categorical_value,proposed_at=now(),challenge_window_ends_at=EXCLUDED.challenge_window_ends_at,attestor_count=EXCLUDED.attestor_count,required_quorum=EXCLUDED.required_quorum,updated_at=now()")
            .bind(&input.event_ticker).bind(numeric).bind(&input.categorical_value).bind(challenge_end).bind(self.quorum).execute(&mut *tx).await.map_err(internal)?;
        tx.commit().await.map_err(internal)?;
        Ok(Response::new(
            self.load_resolution(&input.event_ticker)
                .await
                .map_err(internal)?,
        ))
    }

    async fn finalize_resolution(
        &self,
        request: Request<FinalizeResolutionRequest>,
    ) -> Result<Response<Resolution>, Status> {
        let event = request.into_inner().event_ticker;
        require_text(&event, "event_ticker")?;
        let mut tx = self.pool.begin().await.map_err(internal)?;
        let resolution = sqlx::query("SELECT status::text AS status,attestor_count,required_quorum,challenge_window_ends_at FROM oracle.resolutions WHERE event_ticker=$1 FOR UPDATE").bind(&event).fetch_optional(&mut *tx).await.map_err(internal)?.ok_or_else(|| Status::not_found("resolution not found"))?;
        let status: String = resolution.get("status");
        if status == "FINALIZED" {
            tx.commit().await.map_err(internal)?;
            return Ok(Response::new(
                self.load_resolution(&event).await.map_err(internal)?,
            ));
        }
        if status != "PROPOSED" {
            return Err(Status::failed_precondition("resolution is not proposed"));
        }
        let count: i32 = resolution.get("attestor_count");
        let quorum: i32 = resolution.get("required_quorum");
        if count < quorum {
            return Err(Status::failed_precondition("oracle quorum is not met"));
        }
        let challenge_end: Option<DateTime<Utc>> = resolution.get("challenge_window_ends_at");
        if challenge_end.is_some_and(|end| end > Utc::now()) {
            return Err(Status::failed_precondition(
                "oracle challenge window is open",
            ));
        }
        let values = sqlx::query("SELECT numeric_value,categorical_value FROM oracle.attestations WHERE event_ticker=$1 ORDER BY id").bind(&event).fetch_all(&mut *tx).await.map_err(internal)?;
        let first = values
            .first()
            .ok_or_else(|| Status::failed_precondition("no attestations"))?;
        let first_numeric: Option<i64> = first.try_get("numeric_value").map_err(internal)?;
        let first_category: Option<String> =
            first.try_get("categorical_value").map_err(internal)?;
        for row in values.iter().skip(1) {
            if row
                .try_get::<Option<i64>, _>("numeric_value")
                .map_err(internal)?
                != first_numeric
                || row
                    .try_get::<Option<String>, _>("categorical_value")
                    .map_err(internal)?
                    != first_category
            {
                sqlx::query("UPDATE oracle.resolutions SET status='DISPUTED'::oracle.resolution_status,updated_at=now() WHERE event_ticker=$1").bind(&event).execute(&mut *tx).await.map_err(internal)?;
                tx.commit().await.map_err(internal)?;
                return Err(Status::failed_precondition("oracle attestations conflict"));
            }
        }
        sqlx::query("UPDATE oracle.resolutions SET status='FINALIZED'::oracle.resolution_status,finalized_at=now(),updated_at=now() WHERE event_ticker=$1").bind(&event).execute(&mut *tx).await.map_err(internal)?;
        tx.commit().await.map_err(internal)?;
        let result = self.load_resolution(&event).await.map_err(internal)?;
        self.publish(&event, &result).await;
        Ok(Response::new(result))
    }

    async fn get_resolution(
        &self,
        request: Request<GetResolutionRequest>,
    ) -> Result<Response<Resolution>, Status> {
        Ok(Response::new(
            self.load_resolution(&request.into_inner().event_ticker)
                .await
                .map_err(internal)?,
        ))
    }

    async fn admin_force_resolution(
        &self,
        request: Request<AdminForceResolutionRequest>,
    ) -> Result<Response<Resolution>, Status> {
        let input = request.into_inner();
        require_text(&input.admin_user_id, "admin_user_id")?;
        require_text(&input.justification, "justification")?;
        validate_value(&input.event_ticker, &input.categorical_value)?;
        let numeric = if input.categorical_value.is_empty() {
            Some(input.numeric_value)
        } else {
            None
        };
        sqlx::query("INSERT INTO oracle.resolutions (event_ticker,status,numeric_value,categorical_value,proposed_at,finalized_at,attestor_count,required_quorum) VALUES ($1,'FINALIZED'::oracle.resolution_status,$2,NULLIF($3,''),now(),now(),1,1) ON CONFLICT (event_ticker) DO UPDATE SET status='FINALIZED'::oracle.resolution_status,numeric_value=EXCLUDED.numeric_value,categorical_value=EXCLUDED.categorical_value,finalized_at=now(),updated_at=now()")
            .bind(&input.event_ticker).bind(numeric).bind(&input.categorical_value).execute(&self.pool).await.map_err(internal)?;
        let result = self
            .load_resolution(&input.event_ticker)
            .await
            .map_err(internal)?;
        self.publish(&input.event_ticker, &result).await;
        Ok(Response::new(result))
    }
}

impl OracleService {
    async fn load_resolution(&self, event: &str) -> Result<Resolution> {
        let row = sqlx::query("SELECT status::text AS status,numeric_value,categorical_value,proposed_at,finalized_at FROM oracle.resolutions WHERE event_ticker=$1").bind(event).fetch_optional(&self.pool).await?.context("resolution not found")?;
        let status = match row.get::<String, _>("status").as_str() {
            "PENDING" => ResolutionStatus::Pending,
            "PROPOSED" => ResolutionStatus::Proposed,
            "FINALIZED" => ResolutionStatus::Finalized,
            "DISPUTED" => ResolutionStatus::Disputed,
            _ => ResolutionStatus::Unspecified,
        };
        let rows = sqlx::query("SELECT attestor_id,source,COALESCE(numeric_value,0) AS numeric_value,COALESCE(categorical_value,'') AS categorical_value,signature,observed_at FROM oracle.attestations WHERE event_ticker=$1 ORDER BY id").bind(event).fetch_all(&self.pool).await?;
        Ok(Resolution {
            event_ticker: event.to_owned(),
            numeric_value: row
                .get::<Option<i64>, _>("numeric_value")
                .unwrap_or_default(),
            categorical_value: row
                .get::<Option<String>, _>("categorical_value")
                .unwrap_or_default(),
            status: status as i32,
            attestations: rows
                .into_iter()
                .map(|item| Attestation {
                    attestor_id: item.get("attestor_id"),
                    source: item.get("source"),
                    numeric_value: item.get("numeric_value"),
                    categorical_value: item.get("categorical_value"),
                    signature: item.get("signature"),
                    observed_at: Some(timestamp(item.get::<DateTime<Utc>, _>("observed_at"))),
                })
                .collect(),
            proposed_at: row
                .get::<Option<DateTime<Utc>>, _>("proposed_at")
                .map(timestamp),
            finalized_at: row
                .get::<Option<DateTime<Utc>>, _>("finalized_at")
                .map(timestamp),
        })
    }
    async fn publish(&self, event: &str, resolution: &Resolution) {
        if let Some(publisher) = &self.publisher {
            let payload = serde_json::json!({"event_ticker":event,"status":resolution.status,"numeric_value":resolution.numeric_value,"categorical_value":resolution.categorical_value});
            let _ = publisher
                .publish(
                    format!("oracle.resolutions.finalized.{event}"),
                    serde_json::to_vec(&payload).unwrap_or_default(),
                )
                .await;
        }
    }
}

fn validate_value(event: &str, category: &str) -> Result<(), Status> {
    require_text(event, "event_ticker")?;
    if category.len() > 256 {
        return Err(Status::invalid_argument("categorical_value is too long"));
    }
    Ok(())
}
fn require_text(value: &str, name: &str) -> Result<(), Status> {
    if value.trim().is_empty() {
        Err(Status::invalid_argument(format!("{name} is required")))
    } else {
        Ok(())
    }
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
