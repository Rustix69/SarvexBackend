#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use prost_types::Timestamp;
use sarvex_contracts::sarvex::v1::{
    ledger_client::LedgerClient,
    order_router_server::{OrderRouter, OrderRouterServer},
    ref_data_client::RefDataClient,
    risk_client::RiskClient,
    AddBookRequest, AmendOrderRequest, AmendOrderResponse, CancelOrderRequest, CancelOrderResponse,
    Contract, ContractKind, Fill, FillRecord, GetOrderRequest, ListFillsRequest, ListFillsResponse,
    ListOrdersRequest, ListOrdersResponse, Order, OrderStatus, SubmitOrderRequest,
    SubmitOrderResponse,
};
use sarvex_db::connect;
use sarvex_events::{execution_fills_subject, EventPublisher};
use sarvex_me_client::{flags_for, MeCoreClient};
use sqlx::{postgres::PgPool, QueryBuilder, Row};
use std::{env, net::SocketAddr, time::Duration};
use tonic::{
    transport::{Channel, Endpoint, Server},
    Request, Response, Status,
};
use uuid::Uuid;

#[derive(Clone)]
struct OrderRouterService {
    pool: PgPool,
    refdata: RefDataClient<Channel>,
    risk: RiskClient<Channel>,
    ledger: LedgerClient<Channel>,
    me: MeCoreClient,
}

#[derive(Clone)]
struct HealthState {
    pool: PgPool,
}

const ORDER_SELECT: &str = "SELECT order_id, client_order_id, user_id, ticker, side, action, price_ticks, count, filled_count, tif, post_only, reduce_only, stp, status, reject_code, hold_id, hold_amount_micro_usdc, avg_fill_price_ticks, created_at, updated_at, expires_at FROM orders.orders";

#[tokio::main]
async fn main() -> Result<()> {
    sarvex_runtime::init_tracing("order-router");
    let pool = connect().await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("order-router database readiness check failed")?;
    let refdata = RefDataClient::new(grpc_channel("REFDATA_ADDR", "http://127.0.0.1:50051")?);
    let risk = RiskClient::new(grpc_channel("RISK_ADDR", "http://127.0.0.1:50053")?);
    let ledger = LedgerClient::new(grpc_channel("LEDGER_ADDR", "http://127.0.0.1:50052")?);
    let me_addr = env::var("ME_CORE_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50054".to_owned());
    let timeout_ms = env::var("ME_CORE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(100);
    let me = MeCoreClient::connect_lazy(me_addr, Duration::from_millis(timeout_ms))?;
    let grpc_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("GRPC_PORT").unwrap_or_else(|_| "50055".to_owned())
    )
    .parse()?;
    let http_addr: SocketAddr = format!(
        "0.0.0.0:{}",
        env::var("HTTP_PORT").unwrap_or_else(|_| "8085".to_owned())
    )
    .parse()?;
    let service = OrderRouterService {
        pool: pool.clone(),
        refdata,
        risk,
        ledger,
        me,
    };
    tokio::spawn(run_fill_posting_worker(
        pool.clone(),
        service.ledger.clone(),
        service.refdata.clone(),
    ));
    if let Ok(nats_url) = env::var("NATS_URL") {
        tokio::spawn(run_execution_event_publisher(pool.clone(), nats_url));
    }
    let http = tokio::spawn(run_http(http_addr, HealthState { pool }));
    let grpc = tokio::spawn(
        Server::builder()
            .add_service(OrderRouterServer::new(service))
            .serve_with_shutdown(grpc_addr, shutdown_signal()),
    );
    tokio::select! { result = http => result??, result = grpc => result?? }
    Ok(())
}

fn grpc_channel(name: &str, default: &str) -> Result<Channel> {
    let endpoint = Endpoint::from_shared(env::var(name).unwrap_or_else(|_| default.to_owned()))?;
    Ok(endpoint.connect_lazy())
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
        Json(serde_json::json!({ "status": "ok", "service": "order-router" })),
    )
}
async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ready", "service": "order-router" })),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "status": "not_ready", "error": error.to_string() })),
        ),
    }
}

#[tonic::async_trait]
impl OrderRouter for OrderRouterService {
    async fn submit_order(
        &self,
        request: Request<SubmitOrderRequest>,
    ) -> Result<Response<SubmitOrderResponse>, Status> {
        let request = request.into_inner();
        validate_submit(&request)?;
        if let Some(existing) = self
            .find_order(&request.user_id, None, Some(&request.client_order_id))
            .await?
        {
            return Ok(Response::new(SubmitOrderResponse {
                order: Some(existing),
                reject_code: String::new(),
                reject_reason: String::new(),
                fills: Vec::new(),
            }));
        }
        let order_id = format!("ord_{}", Uuid::new_v4());
        self.insert_pending(&order_id, &request).await?;
        let contract = match self
            .refdata
            .clone()
            .get_contract(sarvex_contracts::sarvex::v1::GetContractRequest {
                ticker: request.ticker.clone(),
            })
            .await
        {
            Ok(contract) => contract.into_inner(),
            Err(error) => {
                self.mark_rejected(&order_id, upstream_code(error.code()))
                    .await?;
                return Ok(Response::new(
                    self.rejected_response(
                        &request.user_id,
                        &order_id,
                        upstream_code(error.code()),
                        error.message(),
                    )
                    .await?,
                ));
            }
        };
        if contract.state != sarvex_contracts::sarvex::v1::ContractState::Open as i32 {
            self.mark_rejected(&order_id, "CONTRACT_NOT_OPEN").await?;
            return Ok(Response::new(
                self.rejected_response(
                    &request.user_id,
                    &order_id,
                    "CONTRACT_NOT_OPEN",
                    "contract is not open",
                )
                .await?,
            ));
        }
        // Book creation is a control-plane operation owned by me-core. The
        // router makes it idempotent for MVP startup: an already-created book
        // is harmless, while submit_order remains the authoritative failure
        // point if me-core is unavailable or the contract is invalid.
        let _ = self
            .me
            .add_book(AddBookRequest {
                ticker: contract.ticker.clone(),
                kind: contract.kind,
                tick_size: contract.tick_size,
                min_price_ticks: contract.min_price_ticks,
                max_price_ticks: contract.max_price_ticks,
            })
            .await;
        let risk = self
            .risk
            .clone()
            .pre_trade_check(sarvex_contracts::sarvex::v1::PreTradeCheckRequest {
                user_id: request.user_id.clone(),
                ticker: request.ticker.clone(),
                side: request.side,
                action: request.action,
                price_ticks: request.price_ticks,
                count: request.count,
            })
            .await
            .map_err(internal)?
            .into_inner();
        if !risk.approved {
            self.mark_rejected(&order_id, &risk.reject_code).await?;
            return Ok(Response::new(
                self.rejected_response(
                    &request.user_id,
                    &order_id,
                    &risk.reject_code,
                    &risk.reject_reason,
                )
                .await?,
            ));
        }
        let hold = self
            .ledger
            .clone()
            .place_hold(sarvex_contracts::sarvex::v1::PlaceHoldRequest {
                idempotency_key: format!("order:{order_id}:hold"),
                user_id: request.user_id.clone(),
                amount_micro_usdc: risk.required_hold_micro_usdc,
                reason: format!("ORDER:{order_id}"),
            })
            .await
            .map_err(internal)?
            .into_inner();
        self.attach_hold(&order_id, &hold.hold_id, risk.required_hold_micro_usdc)
            .await?;
        let me_response = match self
            .me
            .submit_order(sarvex_contracts::sarvex::v1::MeSubmitOrderRequest {
                order_id: order_id.clone(),
                user_id: request.user_id.clone(),
                hold_id: hold.hold_id.clone(),
                ticker: request.ticker.clone(),
                side: request.side,
                action: request.action,
                price_ticks: request.price_ticks,
                count: request.count,
                flags: flags_for(request.tif, request.post_only, request.reduce_only),
                stp: request.stp,
            })
            .await
        {
            Ok(response) => response,
            Err(error) if error.is_unknown_outcome() => {
                tracing::warn!(order_id = %order_id, "matching outcome unknown; retaining PENDING order and hold");
                let order = self
                    .find_order(&request.user_id, Some(&order_id), None)
                    .await?
                    .ok_or_else(|| internal("pending order disappeared"))?;
                return Ok(Response::new(SubmitOrderResponse {
                    order: Some(order),
                    reject_code: "ACK_UNKNOWN".to_owned(),
                    reject_reason: "matching outcome must be reconciled by order_id".to_owned(),
                    fills: Vec::new(),
                }));
            }
            Err(error) if error.was_rejected_before_enqueue() => {
                self.release_hold(&hold.hold_id, risk.required_hold_micro_usdc, &order_id)
                    .await?;
                self.mark_rejected(&order_id, "ME_QUEUE_FULL").await?;
                return Ok(Response::new(
                    self.rejected_response(
                        &request.user_id,
                        &order_id,
                        "ME_QUEUE_FULL",
                        "matching engine queue is full",
                    )
                    .await?,
                ));
            }
            Err(error) => return Err(internal(error.to_string())),
        };
        if !me_response.accepted {
            self.release_hold(&hold.hold_id, risk.required_hold_micro_usdc, &order_id)
                .await?;
            self.mark_rejected(&order_id, &me_response.reject_code)
                .await?;
            return Ok(Response::new(
                self.rejected_response(
                    &request.user_id,
                    &order_id,
                    &me_response.reject_code,
                    "matching engine rejected the order",
                )
                .await?,
            ));
        }
        let fills = self
            .persist_fills(&request, &order_id, &hold.hold_id, &me_response.fills)
            .await?;
        if request.tif == 2 && fills.is_empty() {
            self.release_hold(&hold.hold_id, risk.required_hold_micro_usdc, &order_id)
                .await?;
        }
        let order = self
            .find_order(&request.user_id, Some(&order_id), None)
            .await?
            .ok_or_else(|| internal("accepted order disappeared"))?;
        Ok(Response::new(SubmitOrderResponse {
            order: Some(order),
            fills,
            reject_code: String::new(),
            reject_reason: String::new(),
        }))
    }

    async fn cancel_order(
        &self,
        request: Request<CancelOrderRequest>,
    ) -> Result<Response<CancelOrderResponse>, Status> {
        let request = request.into_inner();
        if request.user_id.trim().is_empty() || request.order_id.trim().is_empty() {
            return Err(Status::invalid_argument(
                "user_id and order_id are required",
            ));
        }
        let existing = self
            .find_order(&request.user_id, Some(&request.order_id), None)
            .await?
            .ok_or_else(|| Status::not_found("order not found"))?;
        if matches!(
            existing.status,
            x if x == OrderStatus::Filled as i32
                || x == OrderStatus::Cancelled as i32
                || x == OrderStatus::Rejected as i32
                || x == OrderStatus::Expired as i32
        ) {
            return Ok(Response::new(CancelOrderResponse {
                order: Some(existing),
                reject_code: "ORDER_TERMINAL".to_owned(),
                reject_reason: "order is already terminal".to_owned(),
            }));
        }
        let me_response = match self
            .me
            .cancel_order(sarvex_contracts::sarvex::v1::MeCancelOrderRequest {
                order_id: request.order_id.clone(),
            })
            .await
        {
            Ok(response) => response,
            Err(error) if error.is_unknown_outcome() => {
                return Ok(Response::new(CancelOrderResponse {
                    order: Some(existing),
                    reject_code: "CANCEL_UNKNOWN".to_owned(),
                    reject_reason: "matching engine cancellation outcome is unknown".to_owned(),
                }))
            }
            Err(error) => return Err(internal(error.to_string())),
        };
        if !me_response.cancelled {
            return Ok(Response::new(CancelOrderResponse {
                order: Some(existing),
                reject_code: me_response.reject_code,
                reject_reason: "matching engine did not cancel the order".to_owned(),
            }));
        }
        sqlx::query("UPDATE orders.orders SET status='CANCELLED', updated_at=now() WHERE order_id=$1 AND user_id=$2 AND status IN ('PENDING','OPEN','PARTIAL')")
            .bind(&request.order_id)
            .bind(&request.user_id)
            .execute(&self.pool)
            .await
            .map_err(internal)?;
        if let Err(error) = self.release_order_remainder(&request.order_id).await {
            tracing::warn!(order_id = %request.order_id, error = %error, "order cancelled but hold release remains pending");
        }
        let order = self
            .find_order(&request.user_id, Some(&request.order_id), None)
            .await?;
        Ok(Response::new(CancelOrderResponse {
            order,
            reject_code: String::new(),
            reject_reason: String::new(),
        }))
    }

    async fn amend_order(
        &self,
        request: Request<AmendOrderRequest>,
    ) -> Result<Response<AmendOrderResponse>, Status> {
        let request = request.into_inner();
        if request.user_id.trim().is_empty() || request.order_id.trim().is_empty() {
            return Err(Status::invalid_argument(
                "user_id and order_id are required",
            ));
        }
        if request.new_price_ticks <= 0 || request.new_count <= 0 {
            return Err(Status::invalid_argument(
                "new price and count must be positive",
            ));
        }
        let existing = self
            .find_order(&request.user_id, Some(&request.order_id), None)
            .await?
            .ok_or_else(|| Status::not_found("order not found"))?;
        if !matches!(
            existing.status,
            x if x == OrderStatus::Open as i32 || x == OrderStatus::Partial as i32
        ) {
            return Ok(Response::new(AmendOrderResponse {
                order: Some(existing),
                reject_code: "ORDER_NOT_AMENDABLE".to_owned(),
            }));
        }
        if request.new_count < existing.filled_count {
            return Ok(Response::new(AmendOrderResponse {
                order: Some(existing),
                reject_code: "COUNT_BELOW_FILLED".to_owned(),
            }));
        }
        let contract = self
            .refdata
            .clone()
            .get_contract(sarvex_contracts::sarvex::v1::GetContractRequest {
                ticker: existing.ticker.clone(),
            })
            .await
            .map_err(internal)?
            .into_inner();
        let required_hold = fill_hold_amount(
            &contract,
            existing.side,
            existing.action,
            request.new_price_ticks,
            request.new_count,
        )?;
        let hold_amount: i64 = sqlx::query_scalar(
            "SELECT hold_amount_micro_usdc FROM orders.orders WHERE order_id=$1 AND user_id=$2",
        )
        .bind(&request.order_id)
        .bind(&request.user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(internal)?;
        if required_hold != hold_amount {
            return Ok(Response::new(AmendOrderResponse {
                order: Some(existing),
                reject_code: "HOLD_RECONCILIATION_REQUIRED".to_owned(),
            }));
        }
        let me_response = match self
            .me
            .amend_order(sarvex_contracts::sarvex::v1::MeAmendOrderRequest {
                order_id: request.order_id.clone(),
                new_price_ticks: request.new_price_ticks,
                new_count: request.new_count,
            })
            .await
        {
            Ok(response) => response,
            Err(error) if error.is_unknown_outcome() => {
                return Ok(Response::new(AmendOrderResponse {
                    order: Some(existing),
                    reject_code: "AMEND_UNKNOWN".to_owned(),
                }))
            }
            Err(error) => return Err(internal(error.to_string())),
        };
        if !me_response.amended {
            return Ok(Response::new(AmendOrderResponse {
                order: Some(existing),
                reject_code: me_response.reject_code,
            }));
        }
        sqlx::query("UPDATE orders.orders SET price_ticks=$1, count=$2, updated_at=now() WHERE order_id=$3 AND user_id=$4 AND status IN ('OPEN','PARTIAL')")
            .bind(request.new_price_ticks)
            .bind(request.new_count)
            .bind(&request.order_id)
            .bind(&request.user_id)
            .execute(&self.pool)
            .await
            .map_err(internal)?;
        let order = self
            .find_order(&request.user_id, Some(&request.order_id), None)
            .await?;
        Ok(Response::new(AmendOrderResponse {
            order,
            reject_code: String::new(),
        }))
    }

    async fn get_order(
        &self,
        request: Request<GetOrderRequest>,
    ) -> Result<Response<Order>, Status> {
        let request = request.into_inner();
        let (order_id, client_order_id) = match request.key {
            Some(sarvex_contracts::sarvex::v1::get_order_request::Key::OrderId(value)) => {
                (Some(value), None)
            }
            Some(sarvex_contracts::sarvex::v1::get_order_request::Key::ClientOrderId(value)) => {
                (None, Some(value))
            }
            None => {
                return Err(Status::invalid_argument(
                    "order_id or client_order_id is required",
                ))
            }
        };
        self.find_order(
            &request.user_id,
            order_id.as_deref(),
            client_order_id.as_deref(),
        )
        .await?
        .map(Response::new)
        .ok_or_else(|| Status::not_found("order not found"))
    }

    async fn list_orders(
        &self,
        request: Request<ListOrdersRequest>,
    ) -> Result<Response<ListOrdersResponse>, Status> {
        let request = request.into_inner();
        if request.user_id.trim().is_empty() {
            return Err(Status::invalid_argument("user_id is required"));
        }
        let limit = request.limit.clamp(1, 100) as i64;
        let mut query = QueryBuilder::new(ORDER_SELECT);
        query.push(" WHERE user_id = ").push_bind(request.user_id);
        if !request.ticker.trim().is_empty() {
            query.push(" AND ticker = ").push_bind(request.ticker);
        }
        if request.status != OrderStatus::Unspecified as i32 {
            query
                .push(" AND status = ")
                .push_bind(status_name(request.status)?);
        }
        query
            .push(" ORDER BY created_at DESC, order_id DESC LIMIT ")
            .push_bind(limit);
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        Ok(Response::new(ListOrdersResponse {
            orders: rows.iter().map(order_from_row).collect(),
            next_cursor: String::new(),
        }))
    }

    async fn list_fills(
        &self,
        request: Request<ListFillsRequest>,
    ) -> Result<Response<ListFillsResponse>, Status> {
        let request = request.into_inner();
        let limit = request.limit.clamp(1, 500) as i64;
        let mut query = QueryBuilder::new("SELECT fill_id, ticker, global_seq, contract_seq, maker_order_id, taker_order_id, maker_user_id, taker_user_id, maker_hold_id, taker_hold_id, maker_side, maker_action, taker_side, taker_action, price_ticks, count, aggressor_side, maker_fee_micro_usdc, taker_fee_micro_usdc, created_at FROM orders.fills WHERE global_seq >= ");
        let cursor_seq = if request.cursor.trim().is_empty() {
            None
        } else {
            Some(
                request
                    .cursor
                    .parse::<u64>()
                    .map_err(|_| Status::invalid_argument("cursor must be a global sequence"))?,
            )
        };
        let from_global_seq = cursor_seq
            .map(|seq| seq.saturating_add(1))
            .unwrap_or(request.from_global_seq);
        query.push_bind(from_global_seq as i64);
        if request.to_global_seq > 0 {
            query
                .push(" AND global_seq <= ")
                .push_bind(request.to_global_seq as i64);
        }
        if !request.ticker.trim().is_empty() {
            query.push(" AND ticker = ").push_bind(request.ticker);
        }
        query
            .push(" ORDER BY global_seq ASC LIMIT ")
            .push_bind(limit);
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        let fills: Vec<_> = rows.iter().map(fill_record_from_row).collect();
        let next_cursor = if fills.len() == limit as usize {
            fills
                .last()
                .map(|fill| fill.global_seq.to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        Ok(Response::new(ListFillsResponse { fills, next_cursor }))
    }
}

impl OrderRouterService {
    async fn release_order_remainder(&self, order_id: &str) -> Result<(), Status> {
        release_order_remainder(&self.pool, &self.ledger, &self.refdata, order_id).await
    }
    async fn insert_pending(
        &self,
        order_id: &str,
        request: &SubmitOrderRequest,
    ) -> Result<(), Status> {
        sqlx::query("INSERT INTO orders.orders (order_id, user_id, client_order_id, ticker, side, action, price_ticks, count, tif, post_only, reduce_only, stp, status, expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'PENDING',$13)").bind(order_id).bind(&request.user_id).bind(&request.client_order_id).bind(&request.ticker).bind(request.side).bind(request.action).bind(request.price_ticks).bind(request.count).bind(request.tif).bind(request.post_only).bind(request.reduce_only).bind(request.stp).bind(request.expires_at.as_ref().and_then(timestamp_to_datetime)).execute(&self.pool).await.map_err(internal)?;
        Ok(())
    }

    async fn attach_hold(&self, order_id: &str, hold_id: &str, amount: i64) -> Result<(), Status> {
        sqlx::query("UPDATE orders.orders SET hold_id=$1, hold_amount_micro_usdc=$2, updated_at=now() WHERE order_id=$3").bind(hold_id).bind(amount).bind(order_id).execute(&self.pool).await.map_err(internal)?;
        Ok(())
    }

    async fn mark_rejected(&self, order_id: &str, code: &str) -> Result<(), Status> {
        sqlx::query("UPDATE orders.orders SET status='REJECTED', reject_code=$1, updated_at=now() WHERE order_id=$2").bind(code).bind(order_id).execute(&self.pool).await.map_err(internal)?;
        Ok(())
    }

    async fn release_hold(&self, hold_id: &str, amount: i64, order_id: &str) -> Result<(), Status> {
        self.ledger
            .clone()
            .release_hold(sarvex_contracts::sarvex::v1::ReleaseHoldRequest {
                idempotency_key: format!("order:{order_id}:hold-release"),
                hold_id: hold_id.to_owned(),
                amount_micro_usdc: amount,
                reason_code: "ORDER_REJECTED".to_owned(),
            })
            .await
            .map_err(internal)?;
        Ok(())
    }

    async fn rejected_response(
        &self,
        user_id: &str,
        order_id: &str,
        code: &str,
        reason: &str,
    ) -> Result<SubmitOrderResponse, Status> {
        let order = self
            .find_order(user_id, Some(order_id), None)
            .await?
            .ok_or_else(|| internal("rejected order disappeared"))?;
        Ok(SubmitOrderResponse {
            order: Some(order),
            fills: Vec::new(),
            reject_code: code.to_owned(),
            reject_reason: reason.to_owned(),
        })
    }

    async fn find_order(
        &self,
        user_id: &str,
        order_id: Option<&str>,
        client_order_id: Option<&str>,
    ) -> Result<Option<Order>, Status> {
        let mut query = QueryBuilder::new(ORDER_SELECT);
        query.push(" WHERE ");
        if let Some(order_id) = order_id {
            query.push("order_id = ").push_bind(order_id);
        } else {
            query.push("user_id = ").push_bind(user_id);
            query
                .push(" AND client_order_id = ")
                .push_bind(client_order_id.unwrap_or_default());
        }
        let row = query
            .build()
            .fetch_optional(&self.pool)
            .await
            .map_err(internal)?;
        Ok(row.as_ref().map(order_from_row))
    }

    async fn persist_fills(
        &self,
        request: &SubmitOrderRequest,
        order_id: &str,
        hold_id: &str,
        fills: &[sarvex_contracts::sarvex::v1::MeFill],
    ) -> Result<Vec<Fill>, Status> {
        let mut tx = self.pool.begin().await.map_err(internal)?;
        let mut response_fills = Vec::new();
        let mut filled_qty = 0_i64;
        let mut notional = 0_i64;
        for fill in fills {
            sqlx::query("INSERT INTO orders.fills (fill_id, ticker, global_seq, contract_seq, maker_order_id, taker_order_id, maker_user_id, taker_user_id, maker_hold_id, taker_hold_id, maker_side, maker_action, taker_side, taker_action, price_ticks, count, aggressor_side, maker_fee_micro_usdc, taker_fee_micro_usdc) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19) ON CONFLICT (fill_id) DO NOTHING").bind(&fill.fill_id).bind(&fill.ticker).bind(fill.global_seq as i64).bind(fill.contract_seq as i64).bind(&fill.maker_order_id).bind(&fill.taker_order_id).bind(&fill.maker_user_id).bind(&fill.taker_user_id).bind(&fill.maker_hold_id).bind(&fill.taker_hold_id).bind(fill.maker_side).bind(fill.maker_action).bind(fill.taker_side).bind(fill.taker_action).bind(fill.price_ticks).bind(fill.count).bind(fill.aggressor_side).bind(fill.maker_fee_micro_usdc).bind(fill.taker_fee_micro_usdc).execute(&mut *tx).await.map_err(internal)?;
            sqlx::query("INSERT INTO orders.fill_posting_outbox (fill_id) VALUES ($1) ON CONFLICT (fill_id) DO NOTHING").bind(&fill.fill_id).execute(&mut *tx).await.map_err(internal)?;
            let subject = execution_fills_subject(&fill.ticker);
            let event_payload = serde_json::json!({
                "schema_version": 1,
                "event_id": fill.fill_id,
                "event_type": "MeFill",
                "subject": subject,
                "global_seq": fill.global_seq,
                "contract_seq": fill.contract_seq,
                "payload": {
                    "ticker": fill.ticker,
                    "maker_order_id": fill.maker_order_id,
                    "taker_order_id": fill.taker_order_id,
                    "maker_user_id": fill.maker_user_id,
                    "taker_user_id": fill.taker_user_id,
                    "maker_hold_id": fill.maker_hold_id,
                    "taker_hold_id": fill.taker_hold_id,
                    "price_ticks": fill.price_ticks,
                    "count": fill.count,
                    "aggressor_side": fill.aggressor_side,
                    "maker_side": fill.maker_side,
                    "maker_action": fill.maker_action,
                    "taker_side": fill.taker_side,
                    "taker_action": fill.taker_action,
                    "maker_fee_micro_usdc": fill.maker_fee_micro_usdc,
                    "taker_fee_micro_usdc": fill.taker_fee_micro_usdc
                }
            });
            sqlx::query("INSERT INTO orders.execution_event_outbox (event_id, subject, event_type, global_seq, contract_seq, payload) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (event_id) DO NOTHING")
                .bind(&fill.fill_id)
                .bind(subject)
                .bind("MeFill")
                .bind(fill.global_seq as i64)
                .bind(fill.contract_seq as i64)
                .bind(event_payload)
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
            update_order_fill(&mut tx, &fill.maker_order_id, fill.count, fill.price_ticks).await?;
            if fill.taker_order_id != fill.maker_order_id {
                update_order_fill(&mut tx, &fill.taker_order_id, fill.count, fill.price_ticks)
                    .await?;
            }
            if fill.taker_order_id == order_id {
                filled_qty += fill.count;
                notional += fill.count.saturating_mul(fill.price_ticks);
                response_fills.push(Fill {
                    fill_id: fill.fill_id.clone(),
                    order_id: order_id.to_owned(),
                    ticker: fill.ticker.clone(),
                    price_ticks: fill.price_ticks,
                    count: fill.count,
                    aggressor_side: fill.aggressor_side,
                    fee_micro_usdc: fill.taker_fee_micro_usdc,
                    ts: fill.ts,
                    seq: fill.global_seq,
                });
            }
        }
        if filled_qty == 0 {
            let status = if request.tif == 2 {
                "CANCELLED"
            } else {
                "OPEN"
            };
            sqlx::query("UPDATE orders.orders SET status=$1, updated_at=now() WHERE order_id=$2")
                .bind(status)
                .bind(order_id)
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
        } else {
            let avg = notional / filled_qty;
            let status = if filled_qty >= request.count {
                "FILLED"
            } else if request.tif == 2 {
                "CANCELLED"
            } else {
                "PARTIAL"
            };
            sqlx::query("UPDATE orders.orders SET avg_fill_price_ticks=$1, status=$2, updated_at=now() WHERE order_id=$3").bind(avg).bind(status).bind(order_id).execute(&mut *tx).await.map_err(internal)?;
        }
        sqlx::query("UPDATE orders.orders SET hold_id=COALESCE(hold_id,$1) WHERE order_id=$2")
            .bind(hold_id)
            .bind(order_id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        tx.commit().await.map_err(internal)?;
        Ok(response_fills)
    }
}

async fn run_execution_event_publisher(pool: PgPool, nats_url: String) {
    loop {
        let publisher = match EventPublisher::connect(&nats_url).await {
            Ok(publisher) => publisher,
            Err(error) => {
                tracing::warn!(error = %error, "NATS connection unavailable; retrying execution event publisher");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        tracing::info!(nats_url = %nats_url, "execution event publisher connected");
        loop {
            let rows = match sqlx::query("SELECT event_id, subject, payload FROM orders.execution_event_outbox WHERE status='PENDING' AND next_attempt_at <= now() ORDER BY global_seq ASC LIMIT 100")
                .fetch_all(&pool)
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(error = %error, "execution event outbox read failed");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            if rows.is_empty() {
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
            for row in rows {
                let event_id: String = row.get("event_id");
                let subject: String = row.get("subject");
                let payload: serde_json::Value = row.get("payload");
                let bytes = match serde_json::to_vec(&payload) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        mark_event_failed(&pool, &event_id, error.to_string()).await;
                        continue;
                    }
                };
                let published: Result<(), String> = publisher
                    .publish(subject, bytes)
                    .await
                    .map_err(|error| error.to_string());
                match published {
                    Ok(()) => {
                        let _ = sqlx::query("UPDATE orders.execution_event_outbox SET status='POSTED', attempts=attempts+1, posted_at=now() WHERE event_id=$1")
                            .bind(&event_id)
                            .execute(&pool)
                            .await;
                    }
                    Err(error) => {
                        tracing::warn!(event_id = %event_id, error = %error, "execution event publish failed");
                        let _ = sqlx::query("UPDATE orders.execution_event_outbox SET attempts=attempts+1, last_error=$2, next_attempt_at=now()+interval '2 seconds' WHERE event_id=$1")
                            .bind(&event_id)
                            .bind(error)
                            .execute(&pool)
                            .await;
                    }
                }
            }
        }
    }
}

async fn run_fill_posting_worker(
    pool: PgPool,
    ledger: LedgerClient<Channel>,
    refdata: RefDataClient<Channel>,
) {
    loop {
        let rows = match sqlx::query(
            "SELECT f.fill_id, f.ticker, f.maker_hold_id, f.taker_hold_id, f.maker_side, f.maker_action, f.taker_side, f.taker_action, f.price_ticks, f.count, f.maker_order_id, f.taker_order_id FROM orders.fill_posting_outbox o JOIN orders.fills f ON f.fill_id=o.fill_id WHERE o.status='PENDING' AND o.next_attempt_at <= now() ORDER BY f.global_seq ASC LIMIT 100",
        )
        .fetch_all(&pool)
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = %error, "fill posting outbox read failed");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if rows.is_empty() {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }
        for row in rows {
            let fill_id: String = row.get("fill_id");
            let result = post_fill(&pool, &ledger, &refdata, &row).await;
            match result {
                Ok(()) => {
                    let _ = sqlx::query("UPDATE orders.fills SET posted=true WHERE fill_id=$1")
                        .bind(&fill_id)
                        .execute(&pool)
                        .await;
                    let _ = sqlx::query("UPDATE orders.fill_posting_outbox SET status='POSTED', attempts=attempts+1, posted_at=now() WHERE fill_id=$1")
                        .bind(&fill_id)
                        .execute(&pool)
                        .await;
                    let maker_order_id: String = row.get("maker_order_id");
                    let taker_order_id: String = row.get("taker_order_id");
                    for order_id in [maker_order_id, taker_order_id] {
                        if let Err(error) =
                            release_order_remainder(&pool, &ledger, &refdata, &order_id).await
                        {
                            tracing::warn!(order_id = %order_id, error = %error, "terminal order remainder release deferred");
                        }
                    }
                }
                Err(error) => {
                    let terminal = is_terminal_fill_posting_error(&error);
                    if terminal {
                        tracing::error!(fill_id = %fill_id, error = %error, "fill ledger posting marked terminally failed");
                    } else {
                        tracing::warn!(fill_id = %fill_id, error = %error, "fill ledger posting deferred");
                    }
                    let _ = sqlx::query("UPDATE orders.fill_posting_outbox SET status=CASE WHEN $3 THEN 'FAILED' ELSE status END, attempts=attempts+1, last_error=$2, next_attempt_at=CASE WHEN $3 THEN now() ELSE now()+interval '2 seconds' END WHERE fill_id=$1")
                        .bind(&fill_id)
                        .bind(error.to_string())
                        .bind(terminal)
                        .execute(&pool)
                        .await;
                }
            }
        }
    }
}

fn is_terminal_fill_posting_error(error: &Status) -> bool {
    error.message().contains("hold is not active")
}

async fn post_fill(
    pool: &PgPool,
    ledger: &LedgerClient<Channel>,
    refdata: &RefDataClient<Channel>,
    row: &sqlx::postgres::PgRow,
) -> Result<(), Status> {
    let fill_id: String = row.get("fill_id");
    let ticker: String = row.get("ticker");
    let contract = tokio::time::timeout(
        Duration::from_secs(3),
        refdata
            .clone()
            .get_contract(sarvex_contracts::sarvex::v1::GetContractRequest {
                ticker: ticker.clone(),
            }),
    )
    .await
    .map_err(|_| Status::deadline_exceeded("refdata contract lookup timed out"))?
    .map_err(internal)?
    .into_inner();
    let destination = format!("LIAB:HOUSE:UNSETTLED_TRADES:{ticker}");
    let maker_hold_id: Option<String> = row.get("maker_hold_id");
    let taker_hold_id: Option<String> = row.get("taker_hold_id");
    let price_ticks: i64 = row.get("price_ticks");
    let count: i64 = row.get("count");
    if let Some(hold_id) = maker_hold_id.filter(|value| !value.trim().is_empty()) {
        let amount = fill_hold_amount(
            &contract,
            row.get("maker_side"),
            row.get("maker_action"),
            price_ticks,
            count,
        )?;
        commit_fill_hold(ledger, &fill_id, "maker", &hold_id, amount, &destination).await?;
    }
    if let Some(hold_id) = taker_hold_id.filter(|value| !value.trim().is_empty()) {
        let amount = fill_hold_amount(
            &contract,
            row.get("taker_side"),
            row.get("taker_action"),
            price_ticks,
            count,
        )?;
        commit_fill_hold(ledger, &fill_id, "taker", &hold_id, amount, &destination).await?;
    }
    let _ = pool;
    Ok(())
}

async fn commit_fill_hold(
    ledger: &LedgerClient<Channel>,
    fill_id: &str,
    party: &str,
    hold_id: &str,
    amount: i64,
    destination: &str,
) -> Result<(), Status> {
    let mut client = ledger.clone();
    tokio::time::timeout(
        Duration::from_secs(3),
        client.commit_hold(sarvex_contracts::sarvex::v1::CommitHoldRequest {
            idempotency_key: format!("fill:{fill_id}:{party}"),
            hold_id: hold_id.to_owned(),
            commit_amount_micro_usdc: amount,
            release_amount_micro_usdc: 0,
            destination_account_code: destination.to_owned(),
            reason_code: "FILL".to_owned(),
            additional_entries: Vec::new(),
        }),
    )
    .await
    .map_err(|_| Status::deadline_exceeded("ledger fill posting timed out"))?
    .map_err(internal)?;
    Ok(())
}

fn fill_hold_amount(
    contract: &Contract,
    side: i32,
    action: i32,
    price: i64,
    count: i64,
) -> Result<i64, Status> {
    if price <= 0 || count <= 0 {
        return Err(Status::invalid_argument(
            "fill price and count must be positive",
        ));
    }
    let amount = if contract.kind == ContractKind::Binary as i32 {
        let risk_ticks = if action == sarvex_contracts::sarvex::v1::Action::Buy as i32 {
            price
        } else {
            contract.max_price_ticks - price + contract.min_price_ticks
        };
        risk_ticks
            .checked_mul(count)
            .and_then(|value| value.checked_mul(10_000))
    } else if contract.kind == ContractKind::Scalar as i32 {
        let signed = signed_position_delta(side, action, count);
        let distance = if signed >= 0 {
            price.checked_sub(contract.lower_bound_ticks)
        } else {
            contract.upper_bound_ticks.checked_sub(price)
        };
        distance
            .and_then(|value| value.checked_mul(count))
            .and_then(|value| value.checked_mul(contract.multiplier_micro_usdc))
    } else {
        None
    };
    amount
        .filter(|value| *value > 0)
        .ok_or_else(|| Status::failed_precondition("invalid fill hold amount"))
}

fn signed_position_delta(side: i32, action: i32, count: i64) -> i64 {
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

async fn release_order_remainder(
    pool: &PgPool,
    ledger: &LedgerClient<Channel>,
    refdata: &RefDataClient<Channel>,
    order_id: &str,
) -> Result<(), Status> {
    let order = sqlx::query("SELECT status, ticker, hold_id, hold_amount_micro_usdc FROM orders.orders WHERE order_id=$1")
        .bind(order_id)
        .fetch_optional(pool)
        .await
        .map_err(internal)?;
    let Some(order) = order else { return Ok(()) };
    let status: String = order.get("status");
    if !matches!(status.as_str(), "FILLED" | "CANCELLED" | "EXPIRED") {
        return Ok(());
    }
    let hold_id: Option<String> = order.get("hold_id");
    let Some(hold_id) = hold_id.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let hold_amount: i64 = order.get("hold_amount_micro_usdc");
    if hold_amount <= 0 {
        return Ok(());
    }
    let ticker: String = order.get("ticker");
    let contract = refdata
        .clone()
        .get_contract(sarvex_contracts::sarvex::v1::GetContractRequest { ticker })
        .await
        .map_err(internal)?
        .into_inner();
    let fills = sqlx::query("SELECT ticker, price_ticks, count, maker_hold_id, taker_hold_id, maker_side, maker_action, taker_side, taker_action FROM orders.fills WHERE maker_order_id=$1 OR taker_order_id=$1 ORDER BY global_seq ASC")
        .bind(order_id)
        .fetch_all(pool)
        .await
        .map_err(internal)?;
    let mut committed = 0_i64;
    for fill in fills {
        let maker_hold: Option<String> = fill.get("maker_hold_id");
        let taker_hold: Option<String> = fill.get("taker_hold_id");
        let (hold, side, action) = if maker_hold.as_deref() == Some(&hold_id) {
            (maker_hold, fill.get("maker_side"), fill.get("maker_action"))
        } else if taker_hold.as_deref() == Some(&hold_id) {
            (taker_hold, fill.get("taker_side"), fill.get("taker_action"))
        } else {
            continue;
        };
        if hold.is_some() {
            committed = committed
                .checked_add(fill_hold_amount(
                    &contract,
                    side,
                    action,
                    fill.get("price_ticks"),
                    fill.get("count"),
                )?)
                .ok_or_else(|| Status::failed_precondition("hold remainder overflow"))?;
        }
    }
    let release = hold_amount.saturating_sub(committed);
    if release == 0 {
        return Ok(());
    }
    let mut client = ledger.clone();
    tokio::time::timeout(
        Duration::from_secs(3),
        client.release_hold(sarvex_contracts::sarvex::v1::ReleaseHoldRequest {
            idempotency_key: format!("order:{order_id}:hold-remainder-release"),
            hold_id,
            amount_micro_usdc: release,
            reason_code: "ORDER_TERMINAL_REMAINDER".to_owned(),
        }),
    )
    .await
    .map_err(|_| Status::deadline_exceeded("ledger hold release timed out"))?
    .map_err(internal)?;
    Ok(())
}

async fn mark_event_failed(pool: &PgPool, event_id: &str, error: String) {
    let _ = sqlx::query("UPDATE orders.execution_event_outbox SET status='FAILED', attempts=attempts+1, last_error=$2 WHERE event_id=$1")
        .bind(event_id)
        .bind(error)
        .execute(pool)
        .await;
}

async fn update_order_fill(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: &str,
    count: i64,
    price: i64,
) -> Result<(), Status> {
    sqlx::query("UPDATE orders.orders SET filled_count=LEAST(orders.orders.count, orders.orders.filled_count+$1), avg_fill_price_ticks=CASE WHEN orders.orders.filled_count+$1 > 0 THEN ((orders.orders.avg_fill_price_ticks * orders.orders.filled_count)+($1*$2))/(orders.orders.filled_count+$1) ELSE 0 END, status=CASE WHEN orders.orders.filled_count+$1 >= orders.orders.count THEN 'FILLED' WHEN orders.orders.filled_count+$1 > 0 THEN 'PARTIAL' ELSE orders.orders.status END, updated_at=now() WHERE order_id=$3").bind(count).bind(price).bind(order_id).execute(&mut **tx).await.map_err(internal)?;
    Ok(())
}

fn order_from_row(row: &sqlx::postgres::PgRow) -> Order {
    let count: i64 = row.get("count");
    let filled_count: i64 = row.get("filled_count");
    Order {
        order_id: row.get("order_id"),
        client_order_id: row.get("client_order_id"),
        user_id: row.get("user_id"),
        ticker: row.get("ticker"),
        side: row.get("side"),
        action: row.get("action"),
        price_ticks: row.get("price_ticks"),
        count,
        filled_count,
        remaining_count: count.saturating_sub(filled_count),
        tif: row.get("tif"),
        post_only: row.get("post_only"),
        reduce_only: row.get("reduce_only"),
        stp: row.get("stp"),
        status: order_status_value(row.get::<String, _>("status").as_str()),
        created_at: Some(timestamp(row.get("created_at"))),
        updated_at: Some(timestamp(row.get("updated_at"))),
        expires_at: row
            .get::<Option<DateTime<Utc>>, _>("expires_at")
            .map(timestamp),
        hold_id: row.get::<Option<String>, _>("hold_id").unwrap_or_default(),
        avg_fill_price_ticks: row.get("avg_fill_price_ticks"),
    }
}

fn fill_record_from_row(row: &sqlx::postgres::PgRow) -> FillRecord {
    FillRecord {
        fill_id: row.get("fill_id"),
        ticker: row.get("ticker"),
        global_seq: row.get::<i64, _>("global_seq") as u64,
        contract_seq: row.get::<i64, _>("contract_seq") as u64,
        maker_order_id: row.get("maker_order_id"),
        taker_order_id: row.get("taker_order_id"),
        maker_user_id: row.get("maker_user_id"),
        taker_user_id: row.get("taker_user_id"),
        maker_hold_id: row
            .get::<Option<String>, _>("maker_hold_id")
            .unwrap_or_default(),
        taker_hold_id: row
            .get::<Option<String>, _>("taker_hold_id")
            .unwrap_or_default(),
        maker_side: row.get("maker_side"),
        maker_action: row.get("maker_action"),
        taker_side: row.get("taker_side"),
        taker_action: row.get("taker_action"),
        price_ticks: row.get("price_ticks"),
        count: row.get("count"),
        aggressor_side: row.get("aggressor_side"),
        maker_fee_micro_usdc: row.get("maker_fee_micro_usdc"),
        taker_fee_micro_usdc: row.get("taker_fee_micro_usdc"),
        ts: Some(timestamp(row.get("created_at"))),
    }
}

fn validate_submit(request: &SubmitOrderRequest) -> Result<(), Status> {
    for (value, name) in [
        (&request.user_id, "user_id"),
        (&request.client_order_id, "client_order_id"),
        (&request.ticker, "ticker"),
        (&request.idempotency_key, "idempotency_key"),
    ] {
        if value.trim().is_empty() {
            return Err(Status::invalid_argument(format!("{name} is required")));
        }
    }
    if request.count <= 0 || request.price_ticks <= 0 {
        return Err(Status::invalid_argument(
            "price_ticks and count must be positive",
        ));
    }
    if request.side == 0 || request.action == 0 {
        return Err(Status::invalid_argument("side and action are required"));
    }
    Ok(())
}

fn status_name(status: i32) -> Result<&'static str, Status> {
    Ok(
        match OrderStatus::try_from(status).unwrap_or(OrderStatus::Unspecified) {
            OrderStatus::Pending => "PENDING",
            OrderStatus::Open => "OPEN",
            OrderStatus::Partial => "PARTIAL",
            OrderStatus::Filled => "FILLED",
            OrderStatus::Cancelled => "CANCELLED",
            OrderStatus::Rejected => "REJECTED",
            OrderStatus::Expired => "EXPIRED",
            OrderStatus::Unspecified => {
                return Err(Status::invalid_argument("invalid order status"))
            }
        },
    )
}
fn order_status_value(value: &str) -> i32 {
    match value {
        "PENDING" => OrderStatus::Pending as i32,
        "OPEN" => OrderStatus::Open as i32,
        "PARTIAL" => OrderStatus::Partial as i32,
        "FILLED" => OrderStatus::Filled as i32,
        "CANCELLED" => OrderStatus::Cancelled as i32,
        "REJECTED" => OrderStatus::Rejected as i32,
        "EXPIRED" => OrderStatus::Expired as i32,
        _ => OrderStatus::Unspecified as i32,
    }
}
fn timestamp(value: DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
    }
}
fn timestamp_to_datetime(value: &Timestamp) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(value.seconds, value.nanos as u32)
}
fn upstream_code(code: tonic::Code) -> &'static str {
    match code {
        tonic::Code::NotFound => "UPSTREAM_NOT_FOUND",
        tonic::Code::InvalidArgument => "UPSTREAM_INVALID_ARGUMENT",
        tonic::Code::FailedPrecondition => "UPSTREAM_FAILED_PRECONDITION",
        tonic::Code::ResourceExhausted => "UPSTREAM_RESOURCE_EXHAUSTED",
        tonic::Code::DeadlineExceeded => "UPSTREAM_DEADLINE_EXCEEDED",
        tonic::Code::Unavailable => "UPSTREAM_UNAVAILABLE",
        _ => "UPSTREAM_ERROR",
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
    fn closed_hold_fill_errors_are_terminal() {
        assert!(is_terminal_fill_posting_error(&Status::internal(
            "status: FailedPrecondition, message: hold is not active",
        )));
        assert!(!is_terminal_fill_posting_error(&Status::unavailable(
            "ledger temporarily unavailable",
        )));
    }
}
