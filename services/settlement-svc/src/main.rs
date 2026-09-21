#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use chrono::Utc;
use sarvex_contracts::sarvex::v1::{
    ledger_client::LedgerClient,
    oracle_client::OracleClient,
    position_client::PositionClient,
    ref_data_client::RefDataClient,
    settlement_server::{Settlement, SettlementServer},
    ContractKind, ContractState, GetContractRequest, GetResolutionRequest, GetSettlementRequest,
    LedgerEntry, ListPositionsByContractRequest, PostTransactionRequest, ResolutionStatus,
    SettleContractRequest, SettlementResult,
};
use sarvex_db::connect;
use sarvex_events::EventPublisher;
use sqlx::{postgres::PgPool, Row};
use std::{env, net::SocketAddr};
use tonic::{
    transport::{Channel, Endpoint, Server},
    Request, Response, Status,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[derive(Clone)]
struct SettlementService {
    pool: PgPool,
    refdata: RefDataClient<Channel>,
    oracle: OracleClient<Channel>,
    positions: PositionClient<Channel>,
    ledger: LedgerClient<Channel>,
    publisher: Option<EventPublisher>,
}
#[derive(Clone)]
struct HealthState {
    pool: PgPool,
}

#[tokio::main]
async fn main() -> Result<()> {
    sarvex_runtime::init_tracing("settlement-svc");
    let pool = connect().await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("settlement database readiness check failed")?;
    let refdata = RefDataClient::new(channel("REFDATA_ADDR", "http://127.0.0.1:50051")?);
    let oracle = OracleClient::new(channel("ORACLE_ADDR", "http://127.0.0.1:50057")?);
    let positions = PositionClient::new(channel("POSITION_ADDR", "http://127.0.0.1:50056")?);
    let ledger = LedgerClient::new(channel("LEDGER_ADDR", "http://127.0.0.1:50052")?);
    let publisher = match env::var("NATS_URL") {
        Ok(url) => Some(EventPublisher::connect(&url).await?),
        Err(_) => None,
    };
    let service = SettlementService {
        pool: pool.clone(),
        refdata,
        oracle,
        positions,
        ledger,
        publisher,
    };
    let grpc_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("GRPC_PORT").unwrap_or_else(|_| "50058".into())
    )
    .parse()?;
    let http_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("HTTP_PORT").unwrap_or_else(|_| "8089".into())
    )
    .parse()?;
    let http = tokio::spawn(run_http(http_addr, HealthState { pool }));
    let grpc = tokio::spawn(
        Server::builder()
            .add_service(SettlementServer::new(service))
            .serve_with_shutdown(grpc_addr, shutdown_signal()),
    );
    tokio::select! { result = http => result??, result = grpc => result?? }
    Ok(())
}

fn channel(name: &str, default: &str) -> Result<Channel> {
    Ok(
        Endpoint::from_shared(env::var(name).unwrap_or_else(|_| default.to_owned()))?
            .connect_lazy(),
    )
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
        Json(serde_json::json!({"status":"ok","service":"settlement-svc"})),
    )
}
async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ready","service":"settlement-svc"})),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status":"not_ready","error":error.to_string()})),
        ),
    }
}

#[tonic::async_trait]
impl Settlement for SettlementService {
    async fn settle_contract(
        &self,
        request: Request<SettleContractRequest>,
    ) -> Result<Response<SettlementResult>, Status> {
        let input = request.into_inner();
        let ticker = require_text(&input.ticker, "ticker")?;
        let contract = self
            .refdata
            .clone()
            .get_contract(GetContractRequest {
                ticker: ticker.clone(),
            })
            .await
            .map_err(internal)?
            .into_inner();
        if contract.close_global_seq == 0 {
            return Err(Status::failed_precondition(
                "contract has no close sequence",
            ));
        }
        if !matches!(
            ContractState::try_from(contract.state),
            Ok(ContractState::Closed | ContractState::Resolving | ContractState::Settled)
        ) {
            return Err(Status::failed_precondition(
                "contract is not closed or resolving",
            ));
        }
        if !input.event_ticker.is_empty() && input.event_ticker != contract.event_ticker {
            return Err(Status::invalid_argument(
                "event_ticker does not belong to contract",
            ));
        }
        let existing = self.existing_settlement(&ticker).await.map_err(internal)?;
        if let Some(existing) = &existing {
            if existing.status == "COMPLETED" {
                return Ok(Response::new(existing.result.clone()));
            }
        }
        self.check_prerequisites(&ticker, contract.close_global_seq)
            .await?;
        let resolution = self
            .oracle
            .clone()
            .get_resolution(GetResolutionRequest {
                event_ticker: if input.event_ticker.is_empty() {
                    contract.event_ticker.clone()
                } else {
                    input.event_ticker.clone()
                },
            })
            .await
            .map_err(internal)?
            .into_inner();
        if ResolutionStatus::try_from(resolution.status).ok() != Some(ResolutionStatus::Finalized) {
            return Err(Status::failed_precondition(
                "oracle resolution is not finalized",
            ));
        }
        let positions = self
            .positions
            .clone()
            .list_positions_by_contract(ListPositionsByContractRequest {
                ticker: ticker.clone(),
                include_closed: true,
                limit: 5000,
                cursor: String::new(),
                min_global_seq: contract.close_global_seq,
            })
            .await
            .map_err(internal)?
            .into_inner()
            .positions;
        let numeric = if resolution.categorical_value.is_empty() {
            resolution.numeric_value
        } else {
            0
        };
        let mut payouts = Vec::with_capacity(positions.len());
        for position in positions {
            let payout = compute_payout(
                &contract,
                &resolution.categorical_value,
                numeric,
                position.net_qty,
            )?;
            payouts.push((position.user_id, position.net_qty, payout));
        }
        let payout_total: i64 = payouts
            .iter()
            .try_fold(0_i64, |sum, (_, _, value)| sum.checked_add(*value))
            .ok_or_else(|| Status::failed_precondition("payout total overflow"))?;
        let escrow = existing
            .as_ref()
            .map(|value| value.escrow_snapshot)
            .filter(|value| *value > 0)
            .unwrap_or(self.house_balance(&ticker).await.map_err(internal)?);
        self.create_intents(
            &ticker,
            &contract.event_ticker,
            contract.close_global_seq,
            numeric,
            &resolution.categorical_value,
            contract.multiplier_micro_usdc,
            escrow,
            &payouts,
        )
        .await?;
        self.post_payouts(&ticker).await?;
        if escrow < payout_total {
            return Err(Status::failed_precondition(
                "ledger escrow is below required payouts",
            ));
        }
        let remainder = escrow - payout_total;
        if remainder > 0 {
            self.post_sweep(&ticker, remainder).await?;
        }
        sqlx::query("UPDATE settlement.settlements SET total_payout_micro_usdc=$2,positions_settled=$3,positions_source_global_seq=$4,rounding_sweep_tx_id=COALESCE(rounding_sweep_tx_id,$5),status='COMPLETED',completed_at=now() WHERE ticker=$1")
            .bind(&ticker).bind(payout_total).bind(payouts.len() as i32).bind(contract.close_global_seq as i64).bind(if remainder > 0 { Some(format!("sweep:{ticker}")) } else { None::<String> }).execute(&self.pool).await.map_err(internal)?;
        sqlx::query("UPDATE refdata.contracts SET state='SETTLED'::refdata.contract_state,updated_at=now() WHERE ticker=$1 AND state <> 'SETTLED'::refdata.contract_state").bind(&ticker).execute(&self.pool).await.map_err(internal)?;
        let result = SettlementResult {
            ticker: ticker.clone(),
            settled_at: Some(prost_types::Timestamp {
                seconds: Utc::now().timestamp(),
                nanos: 0,
            }),
            winner_payout_per_contract_micro_usdc: winner_payout(
                &contract,
                &resolution.categorical_value,
            ),
            total_payout_micro_usdc: payout_total,
            positions_settled: payouts.len() as i32,
        };
        if let Some(publisher) = &self.publisher {
            let _ = publisher.publish(format!("settlement.completed.{ticker}"), serde_json::to_vec(&serde_json::json!({"ticker":ticker,"total_payout_micro_usdc":payout_total})).unwrap_or_default()).await;
        }
        Ok(Response::new(result))
    }

    async fn get_settlement(
        &self,
        request: Request<GetSettlementRequest>,
    ) -> Result<Response<SettlementResult>, Status> {
        let ticker = require_text(&request.into_inner().ticker, "ticker")?;
        let row = sqlx::query("SELECT total_payout_micro_usdc,positions_settled,winner_payout_per_contract_micro_usdc,completed_at FROM settlement.settlements WHERE ticker=$1 AND status='COMPLETED'").bind(&ticker).fetch_optional(&self.pool).await.map_err(internal)?.ok_or_else(|| Status::not_found("settlement not found"))?;
        let completed = row.get::<chrono::DateTime<Utc>, _>("completed_at");
        Ok(Response::new(SettlementResult {
            ticker,
            settled_at: Some(prost_types::Timestamp {
                seconds: completed.timestamp(),
                nanos: completed.timestamp_subsec_nanos() as i32,
            }),
            winner_payout_per_contract_micro_usdc: row.get("winner_payout_per_contract_micro_usdc"),
            total_payout_micro_usdc: row.get("total_payout_micro_usdc"),
            positions_settled: row.get("positions_settled"),
        }))
    }
}

impl SettlementService {
    async fn check_prerequisites(&self, ticker: &str, close_seq: u64) -> Result<(), Status> {
        let unposted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders.fills WHERE ticker=$1 AND global_seq <= $2 AND posted=FALSE").bind(ticker).bind(close_seq as i64).fetch_one(&self.pool).await.map_err(internal)?;
        if unposted != 0 {
            return Err(Status::failed_precondition(
                "fills are not durably posted to ledger",
            ));
        }
        let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders.orders WHERE ticker=$1 AND status IN ('PENDING','OPEN','PARTIAL')").bind(ticker).fetch_one(&self.pool).await.map_err(internal)?;
        if active != 0 {
            return Err(Status::failed_precondition("active orders remain"));
        }
        let max_fill: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(global_seq),0) FROM orders.fills WHERE ticker=$1 AND global_seq <= $2").bind(ticker).bind(close_seq as i64).fetch_one(&self.pool).await.map_err(internal)?;
        let position_seq: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(last_global_seq),0) FROM position.consumer_offsets WHERE stream_name='exec.fills'").fetch_one(&self.pool).await.map_err(internal)?;
        if position_seq < max_fill {
            return Err(Status::failed_precondition(
                "position consumer has not caught up to fills",
            ));
        }
        Ok(())
    }
    async fn existing_settlement(&self, ticker: &str) -> Result<Option<ExistingSettlement>> {
        let row = sqlx::query("SELECT status,ticker,completed_at,winner_payout_per_contract_micro_usdc,total_payout_micro_usdc,positions_settled,escrow_snapshot_micro_usdc FROM settlement.settlements WHERE ticker=$1").bind(ticker).fetch_optional(&self.pool).await?;
        Ok(row.map(|row| ExistingSettlement {
            status: row.get("status"),
            result: SettlementResult {
                ticker: row.get("ticker"),
                settled_at: row
                    .get::<Option<chrono::DateTime<Utc>>, _>("completed_at")
                    .map(|v| prost_types::Timestamp {
                        seconds: v.timestamp(),
                        nanos: v.timestamp_subsec_nanos() as i32,
                    }),
                winner_payout_per_contract_micro_usdc: row
                    .get("winner_payout_per_contract_micro_usdc"),
                total_payout_micro_usdc: row.get("total_payout_micro_usdc"),
                positions_settled: row.get("positions_settled"),
            },
            escrow_snapshot: row.get("escrow_snapshot_micro_usdc"),
        }))
    }
    #[allow(clippy::too_many_arguments)]
    async fn create_intents(
        &self,
        ticker: &str,
        event: &str,
        close_seq: u64,
        numeric: i64,
        category: &str,
        winner: i64,
        escrow: i64,
        payouts: &[(String, i64, i64)],
    ) -> Result<(), Status> {
        let mut tx = self.pool.begin().await.map_err(internal)?;
        sqlx::query("INSERT INTO settlement.settlements (ticker,event_ticker,numeric_value,categorical_value,winner_payout_per_contract_micro_usdc,close_global_seq,escrow_snapshot_micro_usdc,status) VALUES ($1,$2,$3,NULLIF($4,''),$5,$6,$7,'POSTING') ON CONFLICT (ticker) DO UPDATE SET status=CASE WHEN settlement.settlements.status='COMPLETED' THEN settlement.settlements.status ELSE 'POSTING' END").bind(ticker).bind(event).bind(numeric).bind(category).bind(winner).bind(close_seq as i64).bind(escrow).execute(&mut *tx).await.map_err(internal)?;
        for (user, qty, payout) in payouts {
            sqlx::query("INSERT INTO settlement.settlement_payouts (ticker,user_id,position_qty,payout_micro_usdc,idempotency_key) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (idempotency_key) DO NOTHING").bind(ticker).bind(user).bind(*qty).bind(*payout).bind(format!("settlement:{ticker}:{user}")).execute(&mut *tx).await.map_err(internal)?;
        }
        tx.commit().await.map_err(internal)
    }
    async fn post_payouts(&self, ticker: &str) -> Result<(), Status> {
        let rows = sqlx::query("SELECT id,user_id,payout_micro_usdc,idempotency_key FROM settlement.settlement_payouts WHERE ticker=$1 AND status='PENDING' ORDER BY id").bind(ticker).fetch_all(&self.pool).await.map_err(internal)?;
        for row in rows {
            let amount: i64 = row.get("payout_micro_usdc");
            let tx_id = if amount > 0 {
                let mut ledger = self.ledger.clone();
                ledger
                    .post_transaction(PostTransactionRequest {
                        idempotency_key: row.get("idempotency_key"),
                        reason_code: "SETTLEMENT_PAYOUT".into(),
                        entries: vec![
                            LedgerEntry {
                                account_code: format!("LIAB:HOUSE:UNSETTLED_TRADES:{ticker}"),
                                direction: "DR".into(),
                                amount_micro_usdc: amount,
                                memo: "settlement payout".into(),
                            },
                            LedgerEntry {
                                account_code: format!(
                                    "LIAB:USER:{}:CASH",
                                    row.get::<String, _>("user_id")
                                ),
                                direction: "CR".into(),
                                amount_micro_usdc: amount,
                                memo: "settlement payout".into(),
                            },
                        ],
                        metadata: None,
                    })
                    .await
                    .map_err(internal)?
                    .into_inner()
                    .tx_id
            } else {
                String::new()
            };
            sqlx::query("UPDATE settlement.settlement_payouts SET status='POSTED',ledger_tx_id=NULLIF($2,''),posted_at=now() WHERE id=$1").bind(row.get::<i64,_>("id")).bind(tx_id).execute(&self.pool).await.map_err(internal)?;
        }
        Ok(())
    }
    async fn house_balance(&self, ticker: &str) -> Result<i64> {
        let code = format!("LIAB:HOUSE:UNSETTLED_TRADES:{ticker}");
        Ok(sqlx::query_scalar("SELECT COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code=$1 ORDER BY e.entry_id DESC LIMIT 1),0)").bind(code).fetch_one(&self.pool).await?)
    }
    async fn post_sweep(&self, ticker: &str, amount: i64) -> Result<(), Status> {
        let mut ledger = self.ledger.clone();
        ledger
            .post_transaction(PostTransactionRequest {
                idempotency_key: format!("sweep:{ticker}"),
                reason_code: "SETTLEMENT_ROUNDING_SWEEP".into(),
                entries: vec![
                    LedgerEntry {
                        account_code: format!("LIAB:HOUSE:UNSETTLED_TRADES:{ticker}"),
                        direction: "DR".into(),
                        amount_micro_usdc: amount,
                        memo: "settlement rounding sweep".into(),
                    },
                    LedgerEntry {
                        account_code: "REVENUE:SETTLEMENT_ROUNDING".into(),
                        direction: "CR".into(),
                        amount_micro_usdc: amount,
                        memo: "settlement rounding sweep".into(),
                    },
                ],
                metadata: None,
            })
            .await
            .map_err(internal)?;
        Ok(())
    }
}

struct ExistingSettlement {
    status: String,
    result: SettlementResult,
    escrow_snapshot: i64,
}
fn compute_payout(
    contract: &sarvex_contracts::sarvex::v1::Contract,
    category: &str,
    numeric: i64,
    qty: i64,
) -> Result<i64, Status> {
    if qty == 0 {
        return Ok(0);
    }
    let payout = if contract.kind == ContractKind::Binary as i32 {
        let yes = category.eq_ignore_ascii_case("YES") || numeric > 0;
        let winning = if yes { qty.max(0) } else { (-qty).max(0) };
        i128::from(winning).checked_mul(i128::from(contract.multiplier_micro_usdc.max(0)))
    } else {
        let lower = contract.lower_bound_ticks;
        let upper = contract.upper_bound_ticks;
        if upper < lower {
            None
        } else {
            let clamped = numeric.clamp(lower, upper);
            let distance = if qty >= 0 {
                clamped - lower
            } else {
                upper - clamped
            };
            i128::from(qty.unsigned_abs())
                .checked_mul(i128::from(distance))
                .and_then(|v| v.checked_mul(i128::from(contract.multiplier_micro_usdc.max(0))))
        }
    }
    .ok_or_else(|| Status::failed_precondition("settlement payout overflow"))?;
    payout
        .try_into()
        .map_err(|_| Status::failed_precondition("settlement payout exceeds int64"))
}
fn winner_payout(contract: &sarvex_contracts::sarvex::v1::Contract, _category: &str) -> i64 {
    if contract.kind == ContractKind::Binary as i32 {
        contract.multiplier_micro_usdc
    } else {
        0
    }
}
fn require_text(value: &str, name: &str) -> Result<String, Status> {
    if value.trim().is_empty() {
        Err(Status::invalid_argument(format!("{name} is required")))
    } else {
        Ok(value.to_owned())
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
    fn binary_payout_is_integer_and_directional() {
        let contract = sarvex_contracts::sarvex::v1::Contract {
            kind: ContractKind::Binary as i32,
            multiplier_micro_usdc: 1_000_000,
            ..Default::default()
        };
        assert_eq!(compute_payout(&contract, "YES", 0, 3).unwrap(), 3_000_000);
        assert_eq!(compute_payout(&contract, "YES", 0, -3).unwrap(), 0);
    }
    #[test]
    fn scalar_payout_clamps_resolution_to_bounds() {
        let contract = sarvex_contracts::sarvex::v1::Contract {
            kind: ContractKind::Scalar as i32,
            lower_bound_ticks: 10,
            upper_bound_ticks: 20,
            multiplier_micro_usdc: 100,
            ..Default::default()
        };
        assert_eq!(compute_payout(&contract, "", 30, 2).unwrap(), 2_000);
        assert_eq!(compute_payout(&contract, "", 0, -2).unwrap(), 2_000);
    }
}
