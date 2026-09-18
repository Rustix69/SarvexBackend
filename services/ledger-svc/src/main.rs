#![allow(clippy::result_large_err)]

use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use prost_types::{value::Kind, Struct, Timestamp, Value};
use sarvex_contracts::sarvex::v1::{
    ledger_server::{Ledger, LedgerServer},
    AdminCreditDepositRequest, Balance, CommitHoldRequest, GetAccountHistoryRequest,
    GetAccountHistoryResponse, GetBalanceRequest, LedgerEntry, LedgerEntryRecord, PlaceHoldRequest,
    PlaceHoldResponse, PostTransactionRequest, PostTransactionResponse, ReleaseHoldRequest,
};
use sarvex_db::connect;
use serde_json::Value as JsonValue;
use sqlx::{postgres::PgPool, Postgres, Row, Transaction};
use std::{collections::HashMap, env, net::SocketAddr};
use tonic::{transport::Server, Request, Response, Status};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use uuid::Uuid;

#[derive(Clone)]
struct LedgerService {
    pool: PgPool,
}

#[derive(Clone)]
struct HealthState {
    pool: PgPool,
}

#[derive(Clone)]
struct EntrySpec {
    account_code: String,
    direction: String,
    amount: i64,
    memo: String,
}

#[derive(Clone)]
struct AccountState {
    account_id: i64,
    balance: i64,
    next_seq: i64,
}

struct TransactionRecord {
    tx_id: i64,
    posted_at: DateTime<Utc>,
    existed: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    sarvex_runtime::init_tracing("ledger-svc");
    let pool = connect().await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .context("ledger database readiness check failed")?;

    let grpc_port = env::var("GRPC_PORT").unwrap_or_else(|_| "50052".to_owned());
    let http_port = env::var("HTTP_PORT").unwrap_or_else(|_| "8081".to_owned());
    let grpc_addr: SocketAddr = format!("0.0.0.0:{grpc_port}").parse()?;
    let http_addr: SocketAddr = format!("0.0.0.0:{http_port}").parse()?;
    let service = LedgerService { pool: pool.clone() };
    let http_state = HealthState { pool };

    let http = tokio::spawn(run_http(http_addr, http_state));
    let grpc = tokio::spawn(
        Server::builder()
            .add_service(LedgerServer::new(service))
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
        Json(serde_json::json!({ "status": "ok", "service": "ledger-svc" })),
    )
}

async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ready", "service": "ledger-svc" })),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "status": "not_ready", "error": error.to_string() })),
        ),
    }
}

#[tonic::async_trait]
impl Ledger for LedgerService {
    async fn post_transaction(
        &self,
        request: Request<PostTransactionRequest>,
    ) -> Result<Response<PostTransactionResponse>, Status> {
        let request = request.into_inner();
        require_text(&request.idempotency_key, "idempotency_key")?;
        if request.entries.is_empty() {
            return Err(Status::invalid_argument("entries are required"));
        }
        let entries = parse_entries(&request.entries)?;
        ensure_balanced(&entries)?;
        let metadata = struct_to_json(request.metadata.as_ref());
        let mut tx = self.pool.begin().await.map_err(internal)?;
        let record = create_or_get_transaction(
            &mut tx,
            &request.idempotency_key,
            &request.reason_code,
            &metadata,
        )
        .await
        .map_err(internal)?;
        if !record.existed {
            apply_entries(&mut tx, record.tx_id, &entries).await?;
            insert_outbox(&mut tx, record.tx_id, &metadata).await?;
        }
        tx.commit().await.map_err(internal)?;
        Ok(Response::new(PostTransactionResponse {
            tx_id: record.tx_id.to_string(),
            posted_at: Some(timestamp(record.posted_at)),
        }))
    }

    async fn place_hold(
        &self,
        request: Request<PlaceHoldRequest>,
    ) -> Result<Response<PlaceHoldResponse>, Status> {
        let request = request.into_inner();
        require_text(&request.idempotency_key, "idempotency_key")?;
        require_text(&request.user_id, "user_id")?;
        require_positive(request.amount_micro_usdc, "amount_micro_usdc")?;
        let mut tx = self.pool.begin().await.map_err(internal)?;
        if let Some(hold_id) = existing_operation(&mut tx, &request.idempotency_key).await? {
            tx.commit().await.map_err(internal)?;
            return Ok(Response::new(PlaceHoldResponse { hold_id }));
        }
        let cash = user_account(&request.user_id, "CASH");
        let holds = user_account(&request.user_id, "HOLDS");
        ensure_account(&mut tx, &cash, "LIABILITY", Some(&request.user_id)).await?;
        ensure_account(&mut tx, &holds, "LIABILITY", Some(&request.user_id)).await?;
        let hold_id = format!("hold_{}", Uuid::new_v4());
        sqlx::query("INSERT INTO ledger.holds (hold_id, user_id, amount_micro_usdc, reason) VALUES ($1, $2, $3, $4)")
            .bind(&hold_id).bind(&request.user_id).bind(request.amount_micro_usdc).bind(&request.reason)
            .execute(&mut *tx).await.map_err(internal)?;
        let metadata = serde_json::json!({ "operation": "PLACE_HOLD", "hold_id": hold_id });
        let record =
            create_or_get_transaction(&mut tx, &request.idempotency_key, "HOLD_PLACE", &metadata)
                .await
                .map_err(internal)?;
        apply_entries(
            &mut tx,
            record.tx_id,
            &[
                EntrySpec {
                    account_code: cash,
                    direction: "DR".to_owned(),
                    amount: request.amount_micro_usdc,
                    memo: "place hold".to_owned(),
                },
                EntrySpec {
                    account_code: holds,
                    direction: "CR".to_owned(),
                    amount: request.amount_micro_usdc,
                    memo: "place hold".to_owned(),
                },
            ],
        )
        .await?;
        insert_hold_operation(
            &mut tx,
            &request.idempotency_key,
            &hold_id,
            "PLACE",
            request.amount_micro_usdc,
            record.tx_id,
        )
        .await?;
        insert_outbox(&mut tx, record.tx_id, &metadata).await?;
        tx.commit().await.map_err(internal)?;
        Ok(Response::new(PlaceHoldResponse { hold_id }))
    }

    async fn release_hold(
        &self,
        request: Request<ReleaseHoldRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        require_text(&request.idempotency_key, "idempotency_key")?;
        require_text(&request.hold_id, "hold_id")?;
        require_positive(request.amount_micro_usdc, "amount_micro_usdc")?;
        let mut tx = self.pool.begin().await.map_err(internal)?;
        if existing_operation(&mut tx, &request.idempotency_key)
            .await?
            .is_some()
        {
            tx.commit().await.map_err(internal)?;
            return Ok(Response::new(()));
        }
        let hold = lock_hold(&mut tx, &request.hold_id).await?;
        let remaining = hold.amount - hold.committed - hold.released;
        if hold.status != "ACTIVE" {
            return Err(Status::failed_precondition("hold is not active"));
        }
        if request.amount_micro_usdc > remaining {
            return Err(Status::failed_precondition(
                "release exceeds hold remaining",
            ));
        }
        let cash = user_account(&hold.user_id, "CASH");
        let holds = user_account(&hold.user_id, "HOLDS");
        let metadata =
            serde_json::json!({ "operation": "RELEASE_HOLD", "hold_id": request.hold_id });
        let record = create_or_get_transaction(
            &mut tx,
            &request.idempotency_key,
            &request.reason_code,
            &metadata,
        )
        .await
        .map_err(internal)?;
        apply_entries(
            &mut tx,
            record.tx_id,
            &[
                EntrySpec {
                    account_code: holds,
                    direction: "DR".to_owned(),
                    amount: request.amount_micro_usdc,
                    memo: "release hold".to_owned(),
                },
                EntrySpec {
                    account_code: cash,
                    direction: "CR".to_owned(),
                    amount: request.amount_micro_usdc,
                    memo: "release hold".to_owned(),
                },
            ],
        )
        .await?;
        update_hold(&mut tx, &request.hold_id, 0, request.amount_micro_usdc).await?;
        insert_hold_operation(
            &mut tx,
            &request.idempotency_key,
            &request.hold_id,
            "RELEASE",
            request.amount_micro_usdc,
            record.tx_id,
        )
        .await?;
        insert_outbox(&mut tx, record.tx_id, &metadata).await?;
        tx.commit().await.map_err(internal)?;
        Ok(Response::new(()))
    }

    async fn commit_hold(
        &self,
        request: Request<CommitHoldRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        require_text(&request.idempotency_key, "idempotency_key")?;
        require_text(&request.hold_id, "hold_id")?;
        if request.commit_amount_micro_usdc < 0 || request.release_amount_micro_usdc < 0 {
            return Err(Status::invalid_argument(
                "commit and release amounts cannot be negative",
            ));
        }
        let total = request
            .commit_amount_micro_usdc
            .checked_add(request.release_amount_micro_usdc)
            .ok_or_else(|| Status::invalid_argument("amount overflow"))?;
        require_positive(total, "commit_or_release_amount")?;
        if request.commit_amount_micro_usdc > 0 {
            require_text(
                &request.destination_account_code,
                "destination_account_code",
            )?;
        }
        let additional = parse_entries(&request.additional_entries)?;
        let mut tx = self.pool.begin().await.map_err(internal)?;
        if existing_operation(&mut tx, &request.idempotency_key)
            .await?
            .is_some()
        {
            tx.commit().await.map_err(internal)?;
            return Ok(Response::new(()));
        }
        let hold = lock_hold(&mut tx, &request.hold_id).await?;
        let remaining = hold.amount - hold.committed - hold.released;
        if hold.status != "ACTIVE" {
            return Err(Status::failed_precondition("hold is not active"));
        }
        if total > remaining {
            return Err(Status::failed_precondition(
                "commit and release exceed hold remaining",
            ));
        }
        let holds = user_account(&hold.user_id, "HOLDS");
        let cash = user_account(&hold.user_id, "CASH");
        let mut entries = Vec::with_capacity(4 + additional.len());
        if request.commit_amount_micro_usdc > 0 {
            entries.push(EntrySpec {
                account_code: holds.clone(),
                direction: "DR".to_owned(),
                amount: request.commit_amount_micro_usdc,
                memo: "commit hold".to_owned(),
            });
            entries.push(EntrySpec {
                account_code: request.destination_account_code.clone(),
                direction: "CR".to_owned(),
                amount: request.commit_amount_micro_usdc,
                memo: "commit hold".to_owned(),
            });
        }
        if request.release_amount_micro_usdc > 0 {
            entries.push(EntrySpec {
                account_code: holds,
                direction: "DR".to_owned(),
                amount: request.release_amount_micro_usdc,
                memo: "release hold".to_owned(),
            });
            entries.push(EntrySpec {
                account_code: cash,
                direction: "CR".to_owned(),
                amount: request.release_amount_micro_usdc,
                memo: "release hold".to_owned(),
            });
        }
        entries.extend(additional);
        ensure_balanced(&entries)?;
        let metadata =
            serde_json::json!({ "operation": "COMMIT_HOLD", "hold_id": request.hold_id });
        let record = create_or_get_transaction(
            &mut tx,
            &request.idempotency_key,
            &request.reason_code,
            &metadata,
        )
        .await
        .map_err(internal)?;
        apply_entries(&mut tx, record.tx_id, &entries).await?;
        update_hold(
            &mut tx,
            &request.hold_id,
            request.commit_amount_micro_usdc,
            request.release_amount_micro_usdc,
        )
        .await?;
        insert_hold_operation(
            &mut tx,
            &request.idempotency_key,
            &request.hold_id,
            "COMMIT",
            total,
            record.tx_id,
        )
        .await?;
        insert_outbox(&mut tx, record.tx_id, &metadata).await?;
        tx.commit().await.map_err(internal)?;
        Ok(Response::new(()))
    }

    async fn get_balance(
        &self,
        request: Request<GetBalanceRequest>,
    ) -> Result<Response<Balance>, Status> {
        let user_id = request.into_inner().user_id;
        require_text(&user_id, "user_id")?;
        let row = sqlx::query(
            "SELECT cash_micro_usdc, held_micro_usdc FROM ledger.user_balances WHERE user_id = $1",
        )
        .bind(&user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(internal)?;
        let (cash, held) = row
            .map(|row| {
                (
                    row.get::<i64, _>("cash_micro_usdc"),
                    row.get::<i64, _>("held_micro_usdc"),
                )
            })
            .unwrap_or((0, 0));
        Ok(Response::new(Balance {
            user_id,
            cash_micro_usdc: cash,
            held_micro_usdc: held,
            total_micro_usdc: cash + held,
        }))
    }

    async fn get_account_history(
        &self,
        request: Request<GetAccountHistoryRequest>,
    ) -> Result<Response<GetAccountHistoryResponse>, Status> {
        let request = request.into_inner();
        require_text(&request.user_id, "user_id")?;
        let limit = i64::from(request.limit.clamp(1, 500));
        let cursor = request.cursor.parse::<i64>().ok();
        let (sql, has_cursor) = if cursor.is_some() {
            ("SELECT e.entry_id, t.tx_id, a.account_code, e.direction, e.amount_micro_usdc, e.running_balance_micro_usdc, t.reason_code, e.posted_at, COALESCE(e.memo, '') AS memo FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id JOIN ledger.transactions t ON t.tx_id=e.tx_id WHERE a.user_id=$1 AND e.entry_id < $2 ORDER BY e.entry_id DESC LIMIT $3", true)
        } else {
            ("SELECT e.entry_id, t.tx_id, a.account_code, e.direction, e.amount_micro_usdc, e.running_balance_micro_usdc, t.reason_code, e.posted_at, COALESCE(e.memo, '') AS memo FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id JOIN ledger.transactions t ON t.tx_id=e.tx_id WHERE a.user_id=$1 ORDER BY e.entry_id DESC LIMIT $2", false)
        };
        let rows = if has_cursor {
            sqlx::query(sql)
                .bind(&request.user_id)
                .bind(cursor.unwrap())
                .bind(limit + 1)
                .fetch_all(&self.pool)
                .await
                .map_err(internal)?
        } else {
            sqlx::query(sql)
                .bind(&request.user_id)
                .bind(limit + 1)
                .fetch_all(&self.pool)
                .await
                .map_err(internal)?
        };
        let has_next = rows.len() > limit as usize;
        let rows = rows.into_iter().take(limit as usize).collect::<Vec<_>>();
        let next_cursor = if has_next {
            rows.last()
                .map(|row| row.get::<i64, _>("entry_id").to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let entries = rows
            .into_iter()
            .map(|row| LedgerEntryRecord {
                tx_id: row.get::<i64, _>("tx_id").to_string(),
                account_code: row.get("account_code"),
                direction: row.get::<String, _>("direction").trim().to_owned(),
                amount_micro_usdc: row.get("amount_micro_usdc"),
                running_balance_micro_usdc: row.get("running_balance_micro_usdc"),
                reason_code: row.get("reason_code"),
                posted_at: Some(timestamp(row.get("posted_at"))),
                memo: row.get("memo"),
            })
            .collect();
        Ok(Response::new(GetAccountHistoryResponse {
            entries,
            next_cursor,
        }))
    }

    async fn admin_credit_deposit(
        &self,
        request: Request<AdminCreditDepositRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        require_text(&request.user_id, "user_id")?;
        require_positive(request.amount_micro_usdc, "amount_micro_usdc")?;
        let mut tx = self.pool.begin().await.map_err(internal)?;
        let wallet = "ASSET:HOUSE:WALLET".to_owned();
        let cash = user_account(&request.user_id, "CASH");
        let holds = user_account(&request.user_id, "HOLDS");
        ensure_account(&mut tx, &wallet, "ASSET", None).await?;
        ensure_account(&mut tx, &cash, "LIABILITY", Some(&request.user_id)).await?;
        ensure_account(&mut tx, &holds, "LIABILITY", Some(&request.user_id)).await?;
        let idempotency_key = format!("admin_deposit:{}:{}", request.user_id, Uuid::new_v4());
        let metadata =
            serde_json::json!({ "operation": "ADMIN_CREDIT_DEPOSIT", "user_id": request.user_id });
        let record = create_or_get_transaction(&mut tx, &idempotency_key, "DEPOSIT", &metadata)
            .await
            .map_err(internal)?;
        apply_entries(
            &mut tx,
            record.tx_id,
            &[
                EntrySpec {
                    account_code: wallet,
                    direction: "DR".to_owned(),
                    amount: request.amount_micro_usdc,
                    memo: request.note,
                },
                EntrySpec {
                    account_code: cash,
                    direction: "CR".to_owned(),
                    amount: request.amount_micro_usdc,
                    memo: "demo deposit".to_owned(),
                },
            ],
        )
        .await?;
        insert_outbox(&mut tx, record.tx_id, &metadata).await?;
        tx.commit().await.map_err(internal)?;
        Ok(Response::new(()))
    }
}

async fn create_or_get_transaction(
    tx: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    reason_code: &str,
    metadata: &JsonValue,
) -> Result<TransactionRecord, sqlx::Error> {
    let reason = if reason_code.trim().is_empty() {
        "UNSPECIFIED"
    } else {
        reason_code
    };
    let inserted = sqlx::query("INSERT INTO ledger.transactions (idempotency_key, reason_code, metadata) VALUES ($1, $2, $3) ON CONFLICT (idempotency_key) DO NOTHING RETURNING tx_id, posted_at")
        .bind(idempotency_key).bind(reason).bind(metadata).fetch_optional(&mut **tx).await?;
    if let Some(row) = inserted {
        return Ok(TransactionRecord {
            tx_id: row.get("tx_id"),
            posted_at: row.get("posted_at"),
            existed: false,
        });
    }
    let row =
        sqlx::query("SELECT tx_id, posted_at FROM ledger.transactions WHERE idempotency_key=$1")
            .bind(idempotency_key)
            .fetch_one(&mut **tx)
            .await?;
    Ok(TransactionRecord {
        tx_id: row.get("tx_id"),
        posted_at: row.get("posted_at"),
        existed: true,
    })
}

async fn apply_entries(
    tx: &mut Transaction<'_, Postgres>,
    tx_id: i64,
    entries: &[EntrySpec],
) -> Result<(), Status> {
    let mut codes = entries
        .iter()
        .map(|entry| entry.account_code.clone())
        .collect::<Vec<_>>();
    codes.sort();
    codes.dedup();
    let mut states = HashMap::new();
    for code in &codes {
        let row =
            sqlx::query("SELECT account_id FROM ledger.accounts WHERE account_code=$1 FOR UPDATE")
                .bind(code)
                .fetch_one(&mut **tx)
                .await
                .map_err(internal)?;
        let account_id: i64 = row.get("account_id");
        let latest = sqlx::query("SELECT running_balance_micro_usdc, account_seq FROM ledger.entries WHERE account_id=$1 ORDER BY account_seq DESC LIMIT 1").bind(account_id).fetch_optional(&mut **tx).await.map_err(internal)?;
        let (balance, next_seq) = latest
            .map(|row| {
                (
                    row.get::<i64, _>("running_balance_micro_usdc"),
                    row.get::<i64, _>("account_seq"),
                )
            })
            .unwrap_or((0, 0));
        states.insert(
            code.clone(),
            AccountState {
                account_id,
                balance,
                next_seq,
            },
        );
    }
    for entry in entries {
        let state = states
            .get_mut(&entry.account_code)
            .ok_or_else(|| Status::internal("account state missing"))?;
        let balance = if entry.direction == "DR" {
            state.balance.checked_sub(entry.amount)
        } else {
            state.balance.checked_add(entry.amount)
        }
        .ok_or_else(|| Status::failed_precondition("ledger balance overflow"))?;
        if is_user_cash_or_holds(&entry.account_code) && balance < 0 {
            return Err(Status::failed_precondition("insufficient funds"));
        }
        state.next_seq += 1;
        sqlx::query("INSERT INTO ledger.entries (tx_id, account_id, direction, amount_micro_usdc, running_balance_micro_usdc, account_seq, memo) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(tx_id).bind(state.account_id).bind(&entry.direction).bind(entry.amount).bind(balance).bind(state.next_seq).bind(&entry.memo).execute(&mut **tx).await.map_err(internal)?;
        state.balance = balance;
    }
    Ok(())
}

async fn ensure_account(
    tx: &mut Transaction<'_, Postgres>,
    code: &str,
    account_type: &str,
    user_id: Option<&str>,
) -> Result<(), Status> {
    sqlx::query("INSERT INTO ledger.accounts (account_code, account_type, currency, user_id) VALUES ($1, $2::ledger.account_type, 'USDC', $3) ON CONFLICT (account_code) DO NOTHING")
        .bind(code).bind(account_type).bind(user_id).execute(&mut **tx).await.map_err(internal)?;
    Ok(())
}

async fn insert_outbox(
    tx: &mut Transaction<'_, Postgres>,
    tx_id: i64,
    payload: &JsonValue,
) -> Result<(), Status> {
    sqlx::query("INSERT INTO ledger.ledger_event_outbox (tx_id, event_type, payload) VALUES ($1, 'LEDGER_TRANSACTION_POSTED', $2) ON CONFLICT (tx_id, event_type) DO NOTHING")
        .bind(tx_id).bind(payload).execute(&mut **tx).await.map_err(internal)?;
    Ok(())
}

async fn existing_operation(
    tx: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
) -> Result<Option<String>, Status> {
    sqlx::query("SELECT hold_id FROM ledger.hold_operations WHERE idempotency_key=$1")
        .bind(idempotency_key)
        .fetch_optional(&mut **tx)
        .await
        .map(|row| row.map(|row| row.get("hold_id")))
        .map_err(internal)
}

async fn insert_hold_operation(
    tx: &mut Transaction<'_, Postgres>,
    key: &str,
    hold_id: &str,
    operation: &str,
    amount: i64,
    tx_id: i64,
) -> Result<(), Status> {
    sqlx::query("INSERT INTO ledger.hold_operations (idempotency_key, hold_id, operation_type, amount_micro_usdc, ledger_tx_id) VALUES ($1, $2, $3, $4, $5)")
        .bind(key).bind(hold_id).bind(operation).bind(amount).bind(tx_id).execute(&mut **tx).await.map_err(internal)?;
    Ok(())
}

struct HoldState {
    user_id: String,
    amount: i64,
    committed: i64,
    released: i64,
    status: String,
}

async fn lock_hold(tx: &mut Transaction<'_, Postgres>, hold_id: &str) -> Result<HoldState, Status> {
    sqlx::query("SELECT user_id, amount_micro_usdc, committed_micro_usdc, released_micro_usdc, status FROM ledger.holds WHERE hold_id=$1 FOR UPDATE").bind(hold_id).fetch_optional(&mut **tx).await.map_err(internal)?.map(|row| HoldState {
        user_id: row.get("user_id"), amount: row.get("amount_micro_usdc"), committed: row.get("committed_micro_usdc"), released: row.get("released_micro_usdc"), status: row.get("status"),
    }).ok_or_else(|| Status::not_found("hold not found"))
}

async fn update_hold(
    tx: &mut Transaction<'_, Postgres>,
    hold_id: &str,
    committed: i64,
    released: i64,
) -> Result<(), Status> {
    sqlx::query("UPDATE ledger.holds SET committed_micro_usdc=committed_micro_usdc+$1, released_micro_usdc=released_micro_usdc+$2, status=CASE WHEN committed_micro_usdc+released_micro_usdc+$1+$2 >= amount_micro_usdc THEN 'CLOSED' ELSE status END, closed_at=CASE WHEN committed_micro_usdc+released_micro_usdc+$1+$2 >= amount_micro_usdc THEN now() ELSE closed_at END WHERE hold_id=$3")
        .bind(committed).bind(released).bind(hold_id).execute(&mut **tx).await.map_err(internal)?;
    Ok(())
}

fn parse_entries(entries: &[LedgerEntry]) -> Result<Vec<EntrySpec>, Status> {
    entries
        .iter()
        .map(|entry| {
            require_text(&entry.account_code, "account_code")?;
            require_positive(entry.amount_micro_usdc, "entry.amount_micro_usdc")?;
            let direction = entry.direction.trim().to_ascii_uppercase();
            if direction != "DR" && direction != "CR" {
                return Err(Status::invalid_argument("entry.direction must be DR or CR"));
            }
            Ok(EntrySpec {
                account_code: entry.account_code.trim().to_owned(),
                direction,
                amount: entry.amount_micro_usdc,
                memo: entry.memo.clone(),
            })
        })
        .collect()
}

fn ensure_balanced(entries: &[EntrySpec]) -> Result<(), Status> {
    let debit: i128 = entries
        .iter()
        .filter(|entry| entry.direction == "DR")
        .map(|entry| i128::from(entry.amount))
        .sum();
    let credit: i128 = entries
        .iter()
        .filter(|entry| entry.direction == "CR")
        .map(|entry| i128::from(entry.amount))
        .sum();
    if debit != credit {
        return Err(Status::invalid_argument("ledger entries must balance"));
    }
    Ok(())
}

fn user_account(user_id: &str, account: &str) -> String {
    format!("LIAB:USER:{user_id}:{account}")
}
fn is_user_cash_or_holds(code: &str) -> bool {
    code.starts_with("LIAB:USER:") && (code.ends_with(":CASH") || code.ends_with(":HOLDS"))
}
fn require_text(value: &str, field: &str) -> Result<(), Status> {
    if value.trim().is_empty() {
        Err(Status::invalid_argument(format!("{field} is required")))
    } else {
        Ok(())
    }
}
fn require_positive(value: i64, field: &str) -> Result<(), Status> {
    if value <= 0 {
        Err(Status::invalid_argument(format!(
            "{field} must be positive"
        )))
    } else {
        Ok(())
    }
}
fn internal(error: impl std::fmt::Display) -> Status {
    Status::internal(error.to_string())
}
fn timestamp(value: DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
    }
}

fn struct_to_json(value: Option<&Struct>) -> JsonValue {
    value
        .map(|value| {
            JsonValue::Object(
                value
                    .fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value_to_json(value)))
                    .collect(),
            )
        })
        .unwrap_or_else(|| serde_json::json!({}))
}

fn value_to_json(value: &Value) -> JsonValue {
    match value.kind.as_ref() {
        Some(Kind::NullValue(_)) | None => JsonValue::Null,
        Some(Kind::NumberValue(value)) => serde_json::json!(value),
        Some(Kind::StringValue(value)) => JsonValue::String(value.clone()),
        Some(Kind::BoolValue(value)) => JsonValue::Bool(*value),
        Some(Kind::StructValue(value)) => struct_to_json(Some(value)),
        Some(Kind::ListValue(value)) => {
            JsonValue::Array(value.values.iter().map(value_to_json).collect())
        }
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(account_code: &str, direction: &str, amount: i64) -> LedgerEntry {
        LedgerEntry {
            account_code: account_code.to_owned(),
            direction: direction.to_owned(),
            amount_micro_usdc: amount,
            memo: String::new(),
        }
    }

    #[test]
    fn ledger_entries_must_balance() {
        let entries = parse_entries(&[
            entry("ASSET:HOUSE:WALLET", "DR", 100),
            entry("LIAB:USER:u1:CASH", "CR", 100),
        ])
        .expect("valid entries");
        assert!(ensure_balanced(&entries).is_ok());
    }

    #[test]
    fn unbalanced_entries_are_rejected_before_database_work() {
        let entries = parse_entries(&[
            entry("ASSET:HOUSE:WALLET", "DR", 100),
            entry("LIAB:USER:u1:CASH", "CR", 99),
        ])
        .expect("valid individual entries");
        assert_eq!(
            ensure_balanced(&entries).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn invalid_direction_and_amount_are_rejected() {
        assert!(parse_entries(&[entry("A", "XX", 1)]).is_err());
        assert!(parse_entries(&[entry("A", "DR", 0)]).is_err());
    }

    #[test]
    fn user_cash_and_holds_are_non_negative_accounts() {
        assert!(is_user_cash_or_holds("LIAB:USER:u1:CASH"));
        assert!(is_user_cash_or_holds("LIAB:USER:u1:HOLDS"));
        assert!(!is_user_cash_or_holds("ASSET:HOUSE:WALLET"));
    }
}
