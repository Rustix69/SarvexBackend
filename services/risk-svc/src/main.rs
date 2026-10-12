#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
mod margin_exec;
#[cfg(test)]
mod margin_exec_it;
#[cfg(test)]
mod listing_audit_it;
mod portfolio;
mod shadow;
#[cfg(test)]
mod shadow_it;

use portfolio::{ContractRow, EventExposure, EventRow};
use sarvex_contracts::sarvex::v1::{
    ledger_client::LedgerClient,
    ref_data_client::RefDataClient,
    risk_server::{Risk, RiskServer},
    AccountMargin, Action, ApplyFillRequest, ApplyFillResponse, ContractKind, ContractState,
    GetAccountMarginRequest, GetContractRequest, GetUserLimitsRequest, MarginDecision,
    MarginPreview, PreTradeCheckRequest, PreTradeCheckResponse, PreviewMarginRequest,
    ReleaseOrderRequest, ReserveOrderRequest, Side, UpdateUserLimitsRequest, UserLimits,
};
use sarvex_db::connect;
use sqlx::{postgres::PgPool, Row};
use std::collections::BTreeMap;
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
    ledger: LedgerClient<Channel>,
    /// `PORTFOLIO_MARGIN_ENABLED`: accept new PORTFOLIO orders. Fills and releases
    /// for orders already reserved are always processed.
    portfolio_enabled: bool,
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
    let ledger_addr =
        env::var("LEDGER_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50052".to_owned());
    let ledger = LedgerClient::new(Endpoint::from_shared(ledger_addr)?.connect_lazy());
    let portfolio_enabled = env::var("PORTFOLIO_MARGIN_ENABLED")
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes"))
        .unwrap_or(false);
    let grpc_port = env::var("GRPC_PORT").unwrap_or_else(|_| "50053".to_owned());
    let http_port = env::var("HTTP_PORT").unwrap_or_else(|_| "8082".to_owned());
    let grpc_addr: SocketAddr = format!("0.0.0.0:{grpc_port}").parse()?;
    let http_addr: SocketAddr = format!("0.0.0.0:{http_port}").parse()?;
    let service = RiskService {
        pool: pool.clone(),
        refdata,
        ledger,
        portfolio_enabled,
    };
    tokio::spawn(run_margin_reconciler(service.clone()));
    if env::var("MARGIN_SHADOW_LOG_ENABLED")
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes"))
        .unwrap_or(false)
    {
        let secs = env::var("MARGIN_SHADOW_LOG_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|secs| *secs >= 30)
            .unwrap_or(600);
        tracing::info!(interval_secs = secs, "margin shadow logger enabled");
        tokio::spawn(shadow::run_margin_shadow_logger(
            service.clone(),
            std::time::Duration::from_secs(secs),
        ));
    }
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

/// Detects and corrects drift between the ledger's MARGIN accounts and `risk.event_margin`.
async fn run_margin_reconciler(service: RiskService) {
    let mut suspects = std::collections::HashMap::new();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        match service.reconcile_margin(&mut suspects).await {
            Ok(drifts) => {
                for drift in drifts {
                    tracing::warn!(user_id = %drift.user_id, event_ticker = %drift.event_ticker, posted = drift.posted, ledger = drift.ledger_balance, "margin ledger drift seen");
                }
            }
            Err(error) => {
                tracing::warn!(error = %error.message(), "margin reconciliation failed");
            }
        }
    }
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
        let current_position = self
            .current_position(&request.user_id, &request.ticker)
            .await?;
        let working = self
            .working_order_signed_qty(&request.user_id, &request.ticker)
            .await?;
        let effective_position = current_position.saturating_add(working);
        let signed_delta = signed_position_delta(request.side, request.action, request.count);
        let closing_count = if signed_delta < 0 {
            request.count.min(effective_position.max(0))
        } else if signed_delta > 0 {
            request.count.min((-effective_position).max(0))
        } else {
            0
        };
        let opening_count = request.count.saturating_sub(closing_count);
        let required_hold = match required_hold(&request, &contract, opening_count) {
            Ok(value) => value,
            Err(reason) => return Ok(Response::new(reject("INVALID_ORDER", &reason))),
        };
        if required_hold > limits.max_order_size_micro_usdc {
            return Ok(Response::new(reject(
                "MAX_ORDER_NOTIONAL_EXCEEDED",
                "required hold exceeds max order limit",
            )));
        }
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
            opening_qty: opening_count,
            closing_qty: closing_count,
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

    async fn get_account_margin(
        &self,
        request: Request<GetAccountMarginRequest>,
    ) -> Result<Response<AccountMargin>, Status> {
        let user_id = request.into_inner().user_id;
        if user_id.trim().is_empty() {
            return Err(Status::invalid_argument("user_id is required"));
        }
        let exposures = self.load_exposures(&user_id, None).await?;
        let event_tickers: Vec<String> = exposures.keys().cloned().collect();
        let (events, contracts) = self.load_event_refdata(&event_tickers).await?;
        let mut response = AccountMargin {
            user_id,
            mode: MARGIN_MODE_SHADOW.to_owned(),
            ..Default::default()
        };
        for (event_ticker, exposure) in &exposures {
            let event = events
                .get(event_ticker)
                .cloned()
                .unwrap_or_else(|| missing_event(event_ticker));
            let summary = portfolio::summarize(&event, &contracts, exposure);
            if summary.eligible {
                response.total_margin_micro_usdc = response
                    .total_margin_micro_usdc
                    .checked_add(summary.margin_micro_usdc)
                    .ok_or_else(|| Status::internal("margin total overflow"))?;
                response.total_standalone_micro_usdc = response
                    .total_standalone_micro_usdc
                    .checked_add(summary.standalone_micro_usdc)
                    .ok_or_else(|| Status::internal("margin total overflow"))?;
            }
            response.events.push(summary);
        }
        if response.events.iter().any(|e| e.margin_model == "PORTFOLIO") {
            response.mode = MARGIN_MODE_LIVE.to_owned();
        }
        response.savings_micro_usdc =
            response.total_standalone_micro_usdc - response.total_margin_micro_usdc;
        Ok(Response::new(response))
    }

    async fn preview_margin(
        &self,
        request: Request<PreviewMarginRequest>,
    ) -> Result<Response<MarginPreview>, Status> {
        let request = request.into_inner();
        if request.user_id.trim().is_empty() || request.ticker.trim().is_empty() {
            return Err(Status::invalid_argument("user_id and ticker are required"));
        }
        if request.count <= 0 || !valid_side_action(request.side, request.action) {
            return Err(Status::invalid_argument(
                "a valid side, action and positive count are required",
            ));
        }
        let event_ticker: String =
            sqlx::query_scalar("SELECT event_ticker FROM refdata.contracts WHERE ticker=$1")
                .bind(&request.ticker)
                .fetch_optional(&self.pool)
                .await
                .map_err(internal)?
                .ok_or_else(|| Status::not_found("contract not found"))?;
        let mut exposures = self
            .load_exposures(&request.user_id, Some(&event_ticker))
            .await?;
        let before = exposures.remove(&event_ticker).unwrap_or_default();
        let (events, contracts) = self
            .load_event_refdata(std::slice::from_ref(&event_ticker))
            .await?;
        let event = events
            .get(&event_ticker)
            .cloned()
            .unwrap_or_else(|| missing_event(&event_ticker));
        let contract = contracts
            .get(&request.ticker)
            .ok_or_else(|| Status::not_found("contract not found"))?;
        // The preview takes the order as the user enters it (a NO price in NO terms).
        let (unit_qty, unit_price) = portfolio::entered_unit_order(
            contract.kind == "BINARY",
            contract.payout_ticks,
            request.side,
            request.action,
            request.price_ticks,
            request.count,
        );
        let mut after = before.clone();
        after
            .orders
            .push((request.ticker.clone(), unit_qty, unit_price));
        let mut preview = MarginPreview {
            event_ticker: event_ticker.clone(),
            ..Default::default()
        };
        let margin_before = if before.positions.is_empty() && before.orders.is_empty() {
            Ok(None)
        } else {
            portfolio::event_margin(&event, &contracts, &before).map(Some)
        };
        match (
            margin_before,
            portfolio::event_margin(&event, &contracts, &after),
        ) {
            (Ok(before), Ok(after)) => {
                let (margin, standalone) = before
                    .map(|m| (m.margin_micro, m.standalone_micro))
                    .unwrap_or((0, 0));
                preview.eligible = true;
                preview.margin_before_micro_usdc = margin;
                preview.margin_after_micro_usdc = after.margin_micro;
                preview.delta_micro_usdc = after.margin_micro - margin;
                preview.standalone_delta_micro_usdc = after.standalone_micro - standalone;
            }
            (Err(reason), _) | (_, Err(reason)) => {
                preview.eligible = false;
                preview.ineligible_reason = reason;
            }
        }
        Ok(Response::new(preview))
    }

    async fn reserve_order(
        &self,
        request: Request<ReserveOrderRequest>,
    ) -> Result<Response<MarginDecision>, Status> {
        self.reserve_order_impl(request.into_inner())
            .await
            .map(Response::new)
    }

    async fn release_order(
        &self,
        request: Request<ReleaseOrderRequest>,
    ) -> Result<Response<MarginDecision>, Status> {
        self.release_order_impl(request.into_inner())
            .await
            .map(Response::new)
    }

    async fn apply_fill(
        &self,
        request: Request<ApplyFillRequest>,
    ) -> Result<Response<ApplyFillResponse>, Status> {
        self.apply_fill_impl(request.into_inner())
            .await
            .map(Response::new)
    }
}

/// ISOLATED events are only previewed (SHADOW); PORTFOLIO events hold real margin (LIVE).
const MARGIN_MODE_SHADOW: &str = "SHADOW";
const MARGIN_MODE_LIVE: &str = "LIVE";

fn missing_event(event_ticker: &str) -> EventRow {
    EventRow {
        event_ticker: event_ticker.to_owned(),
        margin_model: "UNKNOWN".to_owned(),
        outcome_kind: "UNKNOWN".to_owned(),
        outcome_divider: None,
        grid: sarvex_domain::margin::OutcomeGrid::INTEGERS,
    }
}

struct LimitsRow {
    kyc_tier: i32,
    max_order_size_micro_usdc: i64,
    daily_loss_limit_micro_usdc: i64,
}

impl RiskService {
    /// Today's positions and working orders for one account, grouped by event.
    async fn load_exposures(
        &self,
        user_id: &str,
        event_ticker: Option<&str>,
    ) -> Result<BTreeMap<String, EventExposure>, Status> {
        let mut exposures: BTreeMap<String, EventExposure> = BTreeMap::new();
        let positions = sqlx::query(
            "SELECT p.ticker, c.event_ticker, p.net_qty FROM position.positions p JOIN refdata.contracts c ON c.ticker = p.ticker WHERE p.user_id = $1 AND p.net_qty <> 0 AND ($2::TEXT IS NULL OR c.event_ticker = $2) AND c.event_ticker NOT IN (SELECT event_ticker FROM refdata.events WHERE margin_model = 'PORTFOLIO') ORDER BY p.ticker",
        )
        .bind(user_id)
        .bind(event_ticker)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        for row in positions {
            exposures
                .entry(row.get("event_ticker"))
                .or_default()
                .positions
                .push((row.get("ticker"), row.get("net_qty")));
        }
        let orders = sqlx::query(
            "SELECT o.ticker, c.event_ticker, o.side, o.action, o.price_ticks, GREATEST(o.count - o.filled_count - o.cancelled_count - o.expired_count, 0)::BIGINT AS remaining FROM orders.orders o JOIN refdata.contracts c ON c.ticker = o.ticker WHERE o.user_id = $1 AND o.status IN ('PENDING','OPEN','PARTIAL') AND ($2::TEXT IS NULL OR c.event_ticker = $2) AND c.event_ticker NOT IN (SELECT event_ticker FROM refdata.events WHERE margin_model = 'PORTFOLIO') ORDER BY o.created_at, o.order_id",
        )
        .bind(user_id)
        .bind(event_ticker)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?;
        for row in orders {
            let remaining: i64 = row.get("remaining");
            if remaining <= 0 {
                continue;
            }
            // Stored orders are margined as me-core executes them (see portfolio.rs, CM-05),
            // in time priority so closing capacity is consumed in order (CM-09).
            let (unit_qty, unit_price) = portfolio::stored_unit_order(
                row.get("side"),
                row.get("action"),
                row.get("price_ticks"),
                remaining,
            );
            exposures
                .entry(row.get("event_ticker"))
                .or_default()
                .orders
                .push((row.get("ticker"), unit_qty, unit_price));
        }
        // PORTFOLIO events: risk.margin_positions / risk.margin_orders are authoritative.
        for (event, exposure) in self.load_margin_exposures(user_id, event_ticker).await? {
            exposures.insert(event, exposure);
        }
        Ok(exposures)
    }

    async fn load_event_refdata(
        &self,
        event_tickers: &[String],
    ) -> Result<(BTreeMap<String, EventRow>, BTreeMap<String, ContractRow>), Status> {
        let events = sqlx::query(
            "SELECT event_ticker, margin_model, outcome_kind, outcome_divider, outcome_step, outcome_anchor, outcome_min, outcome_max FROM refdata.events WHERE event_ticker = ANY($1)",
        )
        .bind(event_tickers)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|row| {
            let event = EventRow {
                event_ticker: row.get("event_ticker"),
                margin_model: row.get("margin_model"),
                outcome_kind: row.get("outcome_kind"),
                outcome_divider: row.get("outcome_divider"),
                grid: sarvex_domain::margin::OutcomeGrid {
                    step: row.get("outcome_step"),
                    anchor: row.get("outcome_anchor"),
                    min: row.get("outcome_min"),
                    max: row.get("outcome_max"),
                },
            };
            (event.event_ticker.clone(), event)
        })
        .collect();
        let contracts = sqlx::query(
            "SELECT ticker, event_ticker, kind::text AS kind, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, tick_value_micro, payoff_rule, payout_ticks, divider, COALESCE(settlement_source, '') AS settlement_source, expected_resolution_at::text AS expected_resolution_at, settlement_rule FROM refdata.contracts WHERE event_ticker = ANY($1)",
        )
        .bind(event_tickers)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|row| {
            let contract = ContractRow {
                ticker: row.get("ticker"),
                event_ticker: row.get("event_ticker"),
                kind: row.get("kind"),
                min_price_ticks: row.get("min_price_ticks"),
                max_price_ticks: row.get("max_price_ticks"),
                lower_bound_ticks: row.get("lower_bound_ticks"),
                upper_bound_ticks: row.get("upper_bound_ticks"),
                tick_value_micro: row.get("tick_value_micro"),
                payoff_rule: row.get("payoff_rule"),
                payout_ticks: row.get("payout_ticks"),
                divider: row.get("divider"),
                settlement_source: row.get("settlement_source"),
                // Compared for equality across one event only, so text is enough.
                expected_resolution_at: row.get("expected_resolution_at"),
                settlement_rule: row.get("settlement_rule"),
            };
            (contract.ticker.clone(), contract)
        })
        .collect();
        Ok((events, contracts))
    }

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
    opening_count: i64,
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
        if opening_count == 0 {
            return Ok(0);
        }
        if contract.tick_value_micro <= 0 {
            return Err("scalar tick value is missing".to_owned());
        }
        let is_long = signed_position_delta(request.side, request.action, 1) > 0;
        return sarvex_domain::scalar_lock_micro(
            is_long,
            price,
            opening_count,
            contract.lower_bound_ticks,
            contract.upper_bound_ticks,
            contract.tick_value_micro,
        )
        .map_err(|error| error.to_string());
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
        opening_qty: 0,
        closing_qty: 0,
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
        assert_eq!(
            required_hold(&buy, &contract, buy.count).unwrap(),
            4_000_000
        );
        assert_eq!(
            required_hold(&sell, &contract, sell.count).unwrap(),
            6_000_000
        );
    }

    #[test]
    fn scalar_hold_uses_raw_tick_value_and_opening_quantity() {
        let contract = sarvex_contracts::sarvex::v1::Contract {
            kind: ContractKind::Scalar as i32,
            lower_bound_ticks: 2_000,
            upper_bound_ticks: 8_000,
            tick_value_micro: 100_000,
            ..Default::default()
        };
        let buy = PreTradeCheckRequest {
            side: Side::Long as i32,
            action: Action::Buy as i32,
            price_ticks: 4_000,
            count: 3,
            ..Default::default()
        };
        assert_eq!(required_hold(&buy, &contract, 2).unwrap(), 400_000_000);
        assert_eq!(required_hold(&buy, &contract, 0).unwrap(), 0);
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
