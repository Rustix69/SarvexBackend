#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use sarvex_contracts::sarvex::v1::{
    ref_data_client::RefDataClient,
    risk_server::{Risk, RiskServer},
    Action, ContractKind, ContractState, GetContractRequest, GetUserLimitsRequest,
    PreTradeCheckRequest, PreTradeCheckResponse, Side, UpdateUserLimitsRequest, UserLimits,
};
use sarvex_db::connect;
use sqlx::{postgres::PgPool, Row};
use std::{env, net::SocketAddr};
use tonic::{
    transport::{Channel, Endpoint, Server},
    Request, Response, Status,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[derive(Clone)]
struct RiskService {
    pool: PgPool,
    refdata: RefDataClient<Channel>,
}

#[derive(Clone)]
struct HealthState {
    pool: PgPool,
}

#[tokio::main]
async fn main() -> Result<()> {
    sarvex_runtime::init_tracing("risk-svc");
    let pool = connect().await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("risk database readiness check failed")?;
    let refdata_addr =
        env::var("REFDATA_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50051".to_owned());
    let refdata = RefDataClient::new(Endpoint::from_shared(refdata_addr)?.connect_lazy());
    let grpc_port = env::var("GRPC_PORT").unwrap_or_else(|_| "50053".to_owned());
    let http_port = env::var("HTTP_PORT").unwrap_or_else(|_| "8082".to_owned());
    let grpc_addr: SocketAddr = format!("0.0.0.0:{grpc_port}").parse()?;
    let http_addr: SocketAddr = format!("0.0.0.0:{http_port}").parse()?;
    let service = RiskService {
        pool: pool.clone(),
        refdata,
    };
    let http_state = HealthState { pool };
    let http = tokio::spawn(run_http(http_addr, http_state));
    let grpc = tokio::spawn(
        Server::builder()
            .add_service(RiskServer::new(service))
            .serve_with_shutdown(grpc_addr, shutdown_signal()),
    );
    tokio::select! {
        result = http => result??,
        result = grpc => result??,
    }
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
        Json(serde_json::json!({ "status": "ok", "service": "risk-svc" })),
    )
}

async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ready", "service": "risk-svc" })),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "status": "not_ready", "error": error.to_string() })),
        ),
    }
}

#[tonic::async_trait]
impl Risk for RiskService {
    async fn pre_trade_check(
        &self,
        request: Request<PreTradeCheckRequest>,
    ) -> Result<Response<PreTradeCheckResponse>, Status> {
        let request = request.into_inner();
        if request.user_id.trim().is_empty() || request.ticker.trim().is_empty() {
            return Err(Status::invalid_argument("user_id and ticker are required"));
        }
        if request.count <= 0 {
            return Ok(Response::new(reject(
                "INVALID_QUANTITY",
                "count must be positive",
            )));
        }
        let limits = match self.load_user_limits(&request.user_id).await {
            Ok(limits) => limits,
            Err(error) if error.code() == tonic::Code::NotFound => {
                return Ok(Response::new(reject(
                    "USER_LIMITS_NOT_FOUND",
                    "user limits missing",
                )))
            }
            Err(error) => return Err(error),
        };
        let contract = match self
            .refdata
            .clone()
            .get_contract(GetContractRequest {
                ticker: request.ticker.clone(),
            })
            .await
        {
            Ok(contract) => contract.into_inner(),
            Err(error) if error.code() == tonic::Code::NotFound => {
                return Ok(Response::new(reject(
                    "CONTRACT_NOT_FOUND",
                    "contract not found",
                )))
            }
            Err(error) => return Err(error),
        };
        if contract.state != ContractState::Open as i32 {
            return Ok(Response::new(reject(
                "CONTRACT_NOT_OPEN",
                "contract is not open",
            )));
        }
        if !valid_side_action(request.side, request.action) {
            return Ok(Response::new(reject(
                "INVALID_ORDER",
                "side/action is invalid",
            )));
        }
        if request.price_ticks <= 0
            || request.price_ticks < contract.min_price_ticks
            || request.price_ticks > contract.max_price_ticks
        {
            return Ok(Response::new(reject(
                "INVALID_PRICE",
                "price out of range or market price unsupported",
            )));
        }
        if contract.tick_size <= 0 || request.price_ticks % contract.tick_size != 0 {
            return Ok(Response::new(reject(
                "INVALID_TICK_ALIGNMENT",
                "price not aligned to tick size",
            )));
        }
        if request.count > contract.max_order_size {
            return Ok(Response::new(reject(
                "MAX_ORDER_SIZE_EXCEEDED",
                "count exceeds contract max order size",
            )));
        }
        let required_hold = match required_hold(&request, &contract) {
            Ok(value) => value,
            Err(reason) => return Ok(Response::new(reject("INVALID_ORDER", &reason))),
        };
        if required_hold > limits.max_order_size_micro_usdc {
            return Ok(Response::new(reject(
                "MAX_ORDER_NOTIONAL_EXCEEDED",
                "required hold exceeds max order limit",
            )));
        }
        let current_position = self
            .current_position(&request.user_id, &request.ticker)
            .await?;
        let working = self
            .working_order_signed_qty(&request.user_id, &request.ticker)
            .await?;
        let projected = current_position
            + working
            + signed_position_delta(request.side, request.action, request.count);
        let limit = self
            .position_limit(
                &request.user_id,
                &request.ticker,
                contract.position_limit_per_user,
            )
            .await?;
        if projected.unsigned_abs() > limit.unsigned_abs() {
            return Ok(Response::new(reject(
                "POSITION_LIMIT_EXCEEDED",
                "projected position exceeds limit",
            )));
        }
        Ok(Response::new(PreTradeCheckResponse {
            approved: true,
            required_hold_micro_usdc: required_hold,
            projected_position: projected,
            reject_code: String::new(),
            reject_reason: String::new(),
        }))
    }

    async fn get_user_limits(
        &self,
        request: Request<GetUserLimitsRequest>,
    ) -> Result<Response<UserLimits>, Status> {
        let user_id = request.into_inner().user_id;
        if user_id.trim().is_empty() {
            return Err(Status::invalid_argument("user_id is required"));
        }
        let limits = self.load_user_limits(&user_id).await?;
        let rows = sqlx::query(
            "SELECT ticker, max_qty FROM risk.contract_position_limits WHERE user_id=$1",
        )
        .bind(&user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        let per_contract_position_limit = rows
            .into_iter()
            .map(|row| (row.get("ticker"), row.get("max_qty")))
            .collect();
        Ok(Response::new(UserLimits {
            user_id,
            kyc_tier: limits.kyc_tier,
            max_order_size_micro_usdc: limits.max_order_size_micro_usdc,
            daily_loss_limit_micro_usdc: limits.daily_loss_limit_micro_usdc,
            per_contract_position_limit,
        }))
    }

    async fn update_user_limits(
        &self,
        request: Request<UpdateUserLimitsRequest>,
    ) -> Result<Response<()>, Status> {
        let limits = request
            .into_inner()
            .limits
            .ok_or_else(|| Status::invalid_argument("limits is required"))?;
        if limits.user_id.trim().is_empty()
            || limits.max_order_size_micro_usdc <= 0
            || limits.daily_loss_limit_micro_usdc <= 0
        {
            return Err(Status::invalid_argument(
                "user_id and positive limits are required",
            ));
        }
        let mut tx = self.pool.begin().await.map_err(internal)?;
        sqlx::query("INSERT INTO risk.user_limits (user_id, kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc, updated_at) VALUES ($1,$2,$3,$4,now()) ON CONFLICT (user_id) DO UPDATE SET kyc_tier=EXCLUDED.kyc_tier, max_order_size_micro_usdc=EXCLUDED.max_order_size_micro_usdc, daily_loss_limit_micro_usdc=EXCLUDED.daily_loss_limit_micro_usdc, updated_at=now()")
            .bind(&limits.user_id).bind(limits.kyc_tier).bind(limits.max_order_size_micro_usdc).bind(limits.daily_loss_limit_micro_usdc).execute(&mut *tx).await.map_err(internal)?;
        sqlx::query("DELETE FROM risk.contract_position_limits WHERE user_id=$1")
            .bind(&limits.user_id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        for (ticker, max_qty) in limits.per_contract_position_limit {
            if !ticker.trim().is_empty() && max_qty > 0 {
                sqlx::query("INSERT INTO risk.contract_position_limits (user_id, ticker, max_qty) VALUES ($1,$2,$3)").bind(&limits.user_id).bind(ticker).bind(max_qty).execute(&mut *tx).await.map_err(internal)?;
            }
        }
        tx.commit().await.map_err(internal)?;
        Ok(Response::new(()))
    }
}

struct LimitsRow {
    kyc_tier: i32,
    max_order_size_micro_usdc: i64,
    daily_loss_limit_micro_usdc: i64,
}

impl RiskService {
    async fn load_user_limits(&self, user_id: &str) -> Result<LimitsRow, Status> {
        sqlx::query("SELECT kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc FROM risk.user_limits WHERE user_id=$1")
            .bind(user_id).fetch_optional(&self.pool).await.map_err(internal)?.map(|row| LimitsRow { kyc_tier: row.get("kyc_tier"), max_order_size_micro_usdc: row.get("max_order_size_micro_usdc"), daily_loss_limit_micro_usdc: row.get("daily_loss_limit_micro_usdc") }).ok_or_else(|| Status::not_found("user limits not found"))
    }

    async fn current_position(&self, user_id: &str, ticker: &str) -> Result<i64, Status> {
        Ok(
            sqlx::query("SELECT net_qty FROM position.positions WHERE user_id=$1 AND ticker=$2")
                .bind(user_id)
                .bind(ticker)
                .fetch_optional(&self.pool)
                .await
                .map_err(internal)?
                .map(|row| row.get("net_qty"))
                .unwrap_or(0),
        )
    }

    async fn working_order_signed_qty(&self, user_id: &str, ticker: &str) -> Result<i64, Status> {
        let row = sqlx::query("SELECT COALESCE(SUM(CASE WHEN side IN ('YES','LONG') THEN total_qty ELSE 0 END),0)::BIGINT AS buys, COALESCE(SUM(CASE WHEN side IN ('NO','SHORT') THEN total_qty ELSE 0 END),0)::BIGINT AS sells FROM risk.working_orders_summary WHERE user_id=$1 AND ticker=$2")
            .bind(user_id).bind(ticker).fetch_one(&self.pool).await.map_err(internal)?;
        Ok(row.get::<i64, _>("buys") - row.get::<i64, _>("sells"))
    }

    async fn position_limit(
        &self,
        user_id: &str,
        ticker: &str,
        fallback: i64,
    ) -> Result<i64, Status> {
        Ok(sqlx::query(
            "SELECT max_qty FROM risk.contract_position_limits WHERE user_id=$1 AND ticker=$2",
        )
        .bind(user_id)
        .bind(ticker)
        .fetch_optional(&self.pool)
        .await
        .map_err(internal)?
        .map(|row| row.get("max_qty"))
        .unwrap_or(fallback))
    }
}

fn valid_side_action(side: i32, action: i32) -> bool {
    matches!(side, x if x == Side::Yes as i32 || x == Side::No as i32 || x == Side::Long as i32 || x == Side::Short as i32)
        && matches!(action, x if x == Action::Buy as i32 || x == Action::Sell as i32)
}

fn required_hold(
    request: &PreTradeCheckRequest,
    contract: &sarvex_contracts::sarvex::v1::Contract,
) -> Result<i64, String> {
    let price = request.price_ticks;
    let count = request.count;
    if contract.kind == ContractKind::Binary as i32 {
        let risk_ticks = if request.action == Action::Buy as i32 {
            price
        } else {
            contract.max_price_ticks - price + contract.min_price_ticks
        };
        return risk_ticks
            .checked_mul(count)
            .and_then(|value| value.checked_mul(10_000))
            .ok_or_else(|| "hold calculation overflow".to_owned());
    }
    if contract.kind == ContractKind::Scalar as i32 {
        let multiplier = if contract.multiplier_micro_usdc > 0 {
            contract.multiplier_micro_usdc
        } else {
            return Err("scalar multiplier missing".to_owned());
        };
        let signed = signed_position_delta(request.side, request.action, count);
        let distance = if signed >= 0 {
            price.checked_sub(contract.lower_bound_ticks)
        } else {
            contract.upper_bound_ticks.checked_sub(price)
        }
        .ok_or_else(|| "scalar price is outside bounds".to_owned())?;
        return distance
            .checked_mul(count)
            .and_then(|value| value.checked_mul(multiplier))
            .ok_or_else(|| "hold calculation overflow".to_owned());
    }
    Err("unknown contract kind".to_owned())
}

fn signed_position_delta(side: i32, action: i32, count: i64) -> i64 {
    let side_sign = if side == Side::No as i32 || side == Side::Short as i32 {
        -1
    } else {
        1
    };
    let action_sign = if action == Action::Sell as i32 { -1 } else { 1 };
    count.saturating_mul(side_sign * action_sign)
}

fn reject(code: &str, reason: &str) -> PreTradeCheckResponse {
    PreTradeCheckResponse {
        approved: false,
        reject_code: code.to_owned(),
        reject_reason: reason.to_owned(),
        required_hold_micro_usdc: 0,
        projected_position: 0,
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
    fn binary_buy_and_sell_hold_formulas_are_bounded() {
        let contract = sarvex_contracts::sarvex::v1::Contract {
            kind: ContractKind::Binary as i32,
            min_price_ticks: 1,
            max_price_ticks: 99,
            ..Default::default()
        };
        let buy = PreTradeCheckRequest {
            action: Action::Buy as i32,
            side: Side::Yes as i32,
            price_ticks: 40,
            count: 10,
            ..Default::default()
        };
        let sell = PreTradeCheckRequest {
            action: Action::Sell as i32,
            side: Side::Yes as i32,
            price_ticks: 40,
            count: 10,
            ..Default::default()
        };
        assert_eq!(required_hold(&buy, &contract).unwrap(), 4_000_000);
        assert_eq!(required_hold(&sell, &contract).unwrap(), 6_000_000);
    }

    #[test]
    fn side_and_action_sign_position_deltas() {
        assert_eq!(
            signed_position_delta(Side::Yes as i32, Action::Buy as i32, 5),
            5
        );
        assert_eq!(
            signed_position_delta(Side::Yes as i32, Action::Sell as i32, 5),
            -5
        );
        assert_eq!(
            signed_position_delta(Side::No as i32, Action::Buy as i32, 5),
            -5
        );
        assert_eq!(
            signed_position_delta(Side::No as i32, Action::Sell as i32, 5),
            5
        );
    }

    #[test]
    fn invalid_side_action_is_rejected() {
        assert!(!valid_side_action(0, Action::Buy as i32));
        assert!(!valid_side_action(Side::Yes as i32, 0));
        assert!(valid_side_action(Side::Long as i32, Action::Sell as i32));
    }
}
