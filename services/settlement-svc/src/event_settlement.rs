//! `SettleEvent`: one zero-sum settlement pass for a PORTFOLIO event.
//!
//! Every contract of the event is settled together from `risk.margin_positions`
//! (the authoritative positions of cross margin). For each account the amount is
//! `sum(q_i * V_i(x))`: a loser's amount moves MARGIN -> CLEARING, a winner is paid
//! CLEARING -> CASH, and whatever is left of every MARGIN account is released to CASH.
//! Premiums were already paid at the trades, so nothing else is owed.
//!
//! The pass refuses to run unless the event is fully quiet (CM-10): no working
//! orders, no margin still reserved for orders, every fill applied to margin, and
//! each contract's open interest nets to zero. It is resumable: the positions and
//! amounts are written down first (`settlement.event_settlement_lines` and
//! `settlement.event_settlement_positions`), and every ledger posting has an
//! idempotency key, so a crash is repaired by running `SettleEvent` again.

use sarvex_contracts::sarvex::v1::{
    ledger_client::LedgerClient, EventSettlementResult, LedgerEntry, PostTransactionRequest,
};
use sarvex_domain::margin::{
    listing::parse_payoff_rule, EventBook, Instrument, Outcome, OutcomeGrid, OutcomeKind, Payoff,
    Position,
};
use sqlx::{postgres::PgPool, Row};
use std::collections::BTreeMap;
use tonic::{transport::Channel, Status};

fn internal(error: impl std::fmt::Display) -> Status {
    Status::internal(error.to_string())
}

fn precondition(message: impl Into<String>) -> Status {
    Status::failed_precondition(message.into())
}

/// One account's settlement posting: its margin balance `margin` is released and `net`
/// (positive = receives, negative = pays) is settled against CLEARING.
pub fn settlement_entries(
    event_ticker: &str,
    user_id: &str,
    margin: i64,
    net: i64,
) -> Result<Vec<LedgerEntry>, String> {
    if margin < 0 {
        return Err("margin cannot be negative".to_owned());
    }
    if net < 0 && -net > margin {
        return Err(format!(
            "{user_id} owes {} but only {margin} is posted",
            -net
        ));
    }
    let margin_account = format!("LIAB:USER:{user_id}:MARGIN:{event_ticker}");
    let cash = format!("LIAB:USER:{user_id}:CASH");
    let clearing = format!("LIAB:HOUSE:CLEARING:{event_ticker}");
    let entry = |account: &str, direction: &str, amount: i64, memo: &str| LedgerEntry {
        account_code: account.to_owned(),
        direction: direction.to_owned(),
        amount_micro_usdc: amount,
        memo: memo.to_owned(),
    };
    let mut entries = Vec::new();
    if margin > 0 {
        entries.push(entry(&margin_account, "DR", margin, "release event margin"));
    }
    if net < 0 {
        entries.push(entry(&clearing, "CR", -net, "event settlement loss"));
        if margin + net > 0 {
            entries.push(entry(&cash, "CR", margin + net, "release event margin"));
        }
    } else if net > 0 {
        entries.push(entry(&clearing, "DR", net, "event settlement payout"));
        entries.push(entry(&cash, "CR", margin + net, "event settlement payout"));
    } else if margin > 0 {
        entries.push(entry(&cash, "CR", margin, "release event margin"));
    }
    Ok(entries)
}

struct ContractRow {
    ticker: String,
    state: String,
    instrument: Instrument,
}

fn instrument_for(row: &sqlx::postgres::PgRow) -> Result<Instrument, Status> {
    let ticker: String = row.get("ticker");
    let kind: String = row.get("kind");
    let payoff = match kind.as_str() {
        "SCALAR" => Payoff::Banded {
            lower: row
                .get::<Option<i64>, _>("lower_bound_ticks")
                .ok_or_else(|| precondition(format!("{ticker}: futures band is missing")))?,
            upper: row
                .get::<Option<i64>, _>("upper_bound_ticks")
                .ok_or_else(|| precondition(format!("{ticker}: futures band is missing")))?,
            tick_value_micro: row.get("tick_value_micro"),
        },
        "BINARY" => {
            let rule: serde_json::Value = row
                .get::<Option<serde_json::Value>, _>("payoff_rule")
                .ok_or_else(|| precondition(format!("{ticker}: PAYOFF_RULE_MISSING")))?;
            parse_payoff_rule(&rule, row.get("payout_ticks"))
                .map_err(|reason| precondition(format!("{ticker}: {reason}")))?
        }
        other => {
            return Err(precondition(format!(
                "{ticker}: unsupported contract kind {other}"
            )))
        }
    };
    Ok(Instrument {
        ticker,
        payoff,
        min_price_ticks: row.get("min_price_ticks"),
        max_price_ticks: row.get("max_price_ticks"),
    })
}

async fn margin_balance(pool: &PgPool, user_id: &str, event_ticker: &str) -> Result<i64, Status> {
    sqlx::query_scalar("SELECT COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code=$1 ORDER BY e.entry_id DESC LIMIT 1),0)::BIGINT")
        .bind(format!("LIAB:USER:{user_id}:MARGIN:{event_ticker}"))
        .fetch_one(pool)
        .await
        .map_err(internal)
}

async fn clearing_balance(pool: &PgPool, event_ticker: &str) -> Result<i64, Status> {
    sqlx::query_scalar("SELECT COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code=$1 ORDER BY e.entry_id DESC LIMIT 1),0)::BIGINT")
        .bind(format!("LIAB:HOUSE:CLEARING:{event_ticker}"))
        .fetch_one(pool)
        .await
        .map_err(internal)
}

fn result_of(
    event_ticker: &str,
    status: &str,
    accounts: i32,
    credits: i64,
    debits: i64,
    settled_at: Option<chrono::DateTime<chrono::Utc>>,
) -> EventSettlementResult {
    EventSettlementResult {
        event_ticker: event_ticker.to_owned(),
        status: status.to_owned(),
        accounts_settled: accounts,
        total_credits_micro_usdc: credits,
        total_debits_micro_usdc: debits,
        settled_at: settled_at.map(|at| prost_types::Timestamp {
            seconds: at.timestamp(),
            nanos: at.timestamp_subsec_nanos() as i32,
        }),
    }
}

/// Settles a PORTFOLIO event at `outcome`. Safe to call again after a failure.
pub async fn settle_event(
    pool: &PgPool,
    ledger: &LedgerClient<Channel>,
    event_ticker: &str,
    outcome: Outcome,
) -> Result<EventSettlementResult, Status> {
    if let Some(row) = sqlx::query("SELECT status, accounts_settled, total_credits_micro_usdc, total_debits_micro_usdc, completed_at FROM settlement.event_settlements WHERE event_ticker=$1")
        .bind(event_ticker)
        .fetch_optional(pool)
        .await
        .map_err(internal)?
    {
        if row.get::<String, _>("status") == "COMPLETED" {
            return Ok(result_of(
                event_ticker,
                "COMPLETED",
                row.get("accounts_settled"),
                row.get("total_credits_micro_usdc"),
                row.get("total_debits_micro_usdc"),
                row.get("completed_at"),
            ));
        }
    }

    let event = sqlx::query("SELECT margin_model, outcome_kind, outcome_step, outcome_anchor, outcome_min, outcome_max FROM refdata.events WHERE event_ticker=$1")
        .bind(event_ticker)
        .fetch_optional(pool)
        .await
        .map_err(internal)?
        .ok_or_else(|| Status::not_found("event not found"))?;
    if event.get::<String, _>("margin_model") != "PORTFOLIO" {
        return Err(precondition(
            "SettleEvent is only for PORTFOLIO events; settle ISOLATED events per contract",
        ));
    }
    let kind = match event.get::<String, _>("outcome_kind").as_str() {
        "NUMERIC" => OutcomeKind::Numeric,
        "CATEGORICAL" => OutcomeKind::Categorical,
        other => return Err(precondition(format!("unknown outcome_kind {other}"))),
    };
    let grid = OutcomeGrid {
        step: event.get("outcome_step"),
        anchor: event.get("outcome_anchor"),
        min: event.get("outcome_min"),
        max: event.get("outcome_max"),
    };
    let grid = (kind == OutcomeKind::Numeric && grid != OutcomeGrid::INTEGERS).then_some(grid);

    let mut contracts = Vec::new();
    for row in sqlx::query("SELECT ticker, kind::text AS kind, state::text AS state, lower_bound_ticks, upper_bound_ticks, tick_value_micro, payoff_rule, payout_ticks, min_price_ticks, max_price_ticks FROM refdata.contracts WHERE event_ticker=$1 ORDER BY ticker")
        .bind(event_ticker)
        .fetch_all(pool)
        .await
        .map_err(internal)?
    {
        contracts.push(ContractRow {
            ticker: row.get("ticker"),
            state: row.get("state"),
            instrument: instrument_for(&row)?,
        });
    }
    if contracts.is_empty() {
        return Err(precondition("event has no contracts"));
    }
    if let Some(open) = contracts
        .iter()
        .find(|c| !matches!(c.state.as_str(), "CLOSED" | "RESOLVING" | "SETTLED"))
    {
        return Err(precondition(format!(
            "{} is {}; every contract must be closed before the event settles",
            open.ticker, open.state
        )));
    }
    let book = EventBook::new(
        kind,
        contracts.iter().map(|c| c.instrument.clone()).collect(),
        grid,
    )
    .map_err(|error| precondition(error.to_string()))?;
    book.check_outcome(&outcome)
        .map_err(|error| precondition(error.to_string()))?;

    // The event must be quiet.
    let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders.orders o JOIN refdata.contracts c ON c.ticker=o.ticker WHERE c.event_ticker=$1 AND o.status IN ('PENDING','OPEN','PARTIAL')")
        .bind(event_ticker)
        .fetch_one(pool)
        .await
        .map_err(internal)?;
    if active != 0 {
        return Err(precondition("active orders remain on the event"));
    }
    let reserved: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM risk.margin_orders WHERE event_ticker=$1")
            .bind(event_ticker)
            .fetch_one(pool)
            .await
            .map_err(internal)?;
    if reserved != 0 {
        return Err(precondition("margin is still reserved for working orders"));
    }
    let unapplied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders.fills f JOIN refdata.contracts c ON c.ticker=f.ticker WHERE c.event_ticker=$1 AND NOT EXISTS (SELECT 1 FROM risk.applied_fills a WHERE a.fill_id=f.fill_id)")
        .bind(event_ticker)
        .fetch_one(pool)
        .await
        .map_err(internal)?;
    if unapplied != 0 {
        return Err(precondition(
            "fills are not all applied to margin; retry once the fill worker has caught up",
        ));
    }

    // Positions: the snapshot of an interrupted run, else the live margin positions.
    let have_snapshot: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM settlement.event_settlements WHERE event_ticker=$1)",
    )
    .bind(event_ticker)
    .fetch_one(pool)
    .await
    .map_err(internal)?;
    if !have_snapshot {
        let positions = sqlx::query("SELECT user_id, ticker, net_qty FROM risk.margin_positions WHERE event_ticker=$1 AND net_qty<>0 ORDER BY user_id, ticker")
            .bind(event_ticker)
            .fetch_all(pool)
            .await
            .map_err(internal)?;
        let mut open_interest: BTreeMap<String, i128> = BTreeMap::new();
        let mut per_user: BTreeMap<String, Vec<Position>> = BTreeMap::new();
        for row in &positions {
            let ticker: String = row.get("ticker");
            let qty: i64 = row.get("net_qty");
            *open_interest.entry(ticker.clone()).or_default() += i128::from(qty);
            let index = book
                .instrument_index(&ticker)
                .ok_or_else(|| precondition(format!("{ticker} is not part of the event")))?;
            per_user
                .entry(row.get("user_id"))
                .or_default()
                .push(Position {
                    instrument: index,
                    qty,
                });
        }
        if let Some((ticker, net)) = open_interest.iter().find(|(_, net)| **net != 0) {
            return Err(precondition(format!(
                "open interest of {ticker} does not net to zero ({net}); refusing to settle"
            )));
        }
        let users: Vec<String> = per_user.keys().cloned().collect();
        let accounts: Vec<Vec<Position>> = users.iter().map(|u| per_user[u].clone()).collect();
        let nets = book
            .settle_accounts(&accounts, &outcome)
            .map_err(|error| precondition(error.to_string()))?;
        let credits: i64 = nets.iter().filter(|n| **n > 0).sum();
        let debits: i64 = -nets.iter().filter(|n| **n < 0).sum::<i64>();
        let (numeric, categorical) = match &outcome {
            Outcome::Numeric(x) => (Some(*x), None),
            Outcome::Category(name) => (None, Some(name.clone())),
            Outcome::OtherCategory => (None, Some("OTHER".to_owned())),
        };
        let close_seq: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(f.global_seq),0)::BIGINT FROM orders.fills f JOIN refdata.contracts c ON c.ticker=f.ticker WHERE c.event_ticker=$1")
            .bind(event_ticker)
            .fetch_one(pool)
            .await
            .map_err(internal)?;
        let mut tx = pool.begin().await.map_err(internal)?;
        sqlx::query("INSERT INTO settlement.event_settlements (event_ticker, numeric_value, categorical_value, close_global_seq, total_credits_micro_usdc, total_debits_micro_usdc, accounts_settled, status) VALUES ($1,$2,$3,$4,$5,$6,$7,'POSTING') ON CONFLICT (event_ticker) DO NOTHING")
            .bind(event_ticker)
            .bind(numeric)
            .bind(categorical)
            .bind(close_seq)
            .bind(credits)
            .bind(debits)
            .bind(users.len() as i32)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        for row in &positions {
            sqlx::query("INSERT INTO settlement.event_settlement_positions (event_ticker, user_id, ticker, net_qty) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING")
                .bind(event_ticker)
                .bind(row.get::<String, _>("user_id"))
                .bind(row.get::<String, _>("ticker"))
                .bind(row.get::<i64, _>("net_qty"))
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
        }
        for (user, net) in users.iter().zip(&nets) {
            let posted: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT posted_micro_usdc FROM risk.event_margin WHERE user_id=$1 AND event_ticker=$2),0)")
                .bind(user)
                .bind(event_ticker)
                .fetch_one(&mut *tx)
                .await
                .map_err(internal)?;
            sqlx::query("INSERT INTO settlement.event_settlement_lines (event_ticker, user_id, net_micro_usdc, margin_released_micro_usdc, idempotency_key) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (event_ticker, user_id) DO NOTHING")
                .bind(event_ticker)
                .bind(user)
                .bind(*net)
                .bind(posted)
                .bind(format!("settle-event:{event_ticker}:{user}"))
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
        }
        // Accounts that hold margin but no position (an order that was never filled).
        for row in sqlx::query("SELECT user_id, posted_micro_usdc FROM risk.event_margin WHERE event_ticker=$1 AND posted_micro_usdc>0 AND user_id NOT IN (SELECT user_id FROM settlement.event_settlement_lines WHERE event_ticker=$1)")
            .bind(event_ticker)
            .fetch_all(&mut *tx)
            .await
            .map_err(internal)?
        {
            let user: String = row.get("user_id");
            sqlx::query("INSERT INTO settlement.event_settlement_lines (event_ticker, user_id, net_micro_usdc, margin_released_micro_usdc, idempotency_key) VALUES ($1,$2,0,$3,$4) ON CONFLICT (event_ticker, user_id) DO NOTHING")
                .bind(event_ticker)
                .bind(&user)
                .bind(row.get::<i64, _>("posted_micro_usdc"))
                .bind(format!("settle-event:{event_ticker}:{user}"))
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
        }
        tx.commit().await.map_err(internal)?;
    }

    // Post losers first so CLEARING is funded before winners are paid.
    let lines = sqlx::query("SELECT id, user_id, net_micro_usdc, margin_released_micro_usdc, idempotency_key FROM settlement.event_settlement_lines WHERE event_ticker=$1 AND status='PENDING' ORDER BY net_micro_usdc ASC, id ASC")
        .bind(event_ticker)
        .fetch_all(pool)
        .await
        .map_err(internal)?;
    for line in lines {
        let user: String = line.get("user_id");
        let net: i64 = line.get("net_micro_usdc");
        // The margin released is the amount written down when the settlement was planned,
        // so a retry builds the same entries and the ledger's idempotency key makes it a no-op.
        let margin: i64 = line.get("margin_released_micro_usdc");
        let balance = margin_balance(pool, &user, event_ticker).await?;
        if balance != margin && balance != 0 {
            mark_failed(pool, event_ticker).await;
            return Err(precondition(format!(
                "margin of {user} on {event_ticker}: ledger {balance} != planned {margin}; run the reconciler first"
            )));
        }
        let entries = settlement_entries(event_ticker, &user, margin, net)
            .map_err(|reason| precondition(format!("cannot settle {user}: {reason}")))?;
        let ledger_tx = if entries.is_empty() {
            String::new()
        } else {
            ledger
                .clone()
                .post_transaction(PostTransactionRequest {
                    idempotency_key: line.get("idempotency_key"),
                    reason_code: "EVENT_SETTLEMENT".to_owned(),
                    entries,
                    metadata: None,
                })
                .await
                .map_err(|status| {
                    Status::new(
                        status.code(),
                        format!("settlement posting for {user} failed: {}", status.message()),
                    )
                })?
                .into_inner()
                .tx_id
        };
        let mut tx = pool.begin().await.map_err(internal)?;
        sqlx::query("UPDATE settlement.event_settlement_lines SET status='POSTED', ledger_tx_id=NULLIF($2,''), posted_at=now() WHERE id=$1")
            .bind(line.get::<i64, _>("id"))
            .bind(&ledger_tx)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        sqlx::query("UPDATE risk.event_margin SET margin_micro_usdc=0, position_requirement_micro_usdc=0, posted_micro_usdc=0, standalone_micro_usdc=0, worst_outcome=NULL, version=version+1, updated_at=now() WHERE user_id=$1 AND event_ticker=$2")
            .bind(&user)
            .bind(event_ticker)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        tx.commit().await.map_err(internal)?;
    }

    let clearing = clearing_balance(pool, event_ticker).await?;
    if clearing != 0 {
        mark_failed(pool, event_ticker).await;
        return Err(precondition(format!(
            "CLEARING:{event_ticker} is {clearing} after settlement; investigate before retrying"
        )));
    }
    let mut tx = pool.begin().await.map_err(internal)?;
    sqlx::query("UPDATE risk.margin_positions SET net_qty=0, updated_at=now() WHERE event_ticker=$1")
        .bind(event_ticker)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("UPDATE settlement.event_settlements SET status='COMPLETED', completed_at=now() WHERE event_ticker=$1")
        .bind(event_ticker)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("UPDATE refdata.contracts SET state='SETTLED'::refdata.contract_state, updated_at=now() WHERE event_ticker=$1 AND state <> 'SETTLED'::refdata.contract_state")
        .bind(event_ticker)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let row = sqlx::query("SELECT accounts_settled, total_credits_micro_usdc, total_debits_micro_usdc, completed_at FROM settlement.event_settlements WHERE event_ticker=$1")
        .bind(event_ticker)
        .fetch_one(pool)
        .await
        .map_err(internal)?;
    Ok(result_of(
        event_ticker,
        "COMPLETED",
        row.get("accounts_settled"),
        row.get("total_credits_micro_usdc"),
        row.get("total_debits_micro_usdc"),
        row.get("completed_at"),
    ))
}

async fn mark_failed(pool: &PgPool, event_ticker: &str) {
    let _ = sqlx::query("UPDATE settlement.event_settlements SET status='FAILED' WHERE event_ticker=$1 AND status<>'COMPLETED'")
        .bind(event_ticker)
        .execute(pool)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(entries: &[LedgerEntry], direction: &str) -> i64 {
        entries
            .iter()
            .filter(|e| e.direction == direction)
            .map(|e| e.amount_micro_usdc)
            .sum()
    }

    #[test]
    fn a_loser_pays_from_margin_and_gets_the_rest_back() {
        let entries = settlement_entries("EV", "bob", 2000, -1000).unwrap();
        assert_eq!(total(&entries, "DR"), total(&entries, "CR"));
        assert!(entries.iter().any(|e| e.account_code == "LIAB:HOUSE:CLEARING:EV"
            && e.direction == "CR"
            && e.amount_micro_usdc == 1000));
        assert!(entries.iter().any(|e| e.account_code == "LIAB:USER:bob:CASH"
            && e.direction == "CR"
            && e.amount_micro_usdc == 1000));
    }

    #[test]
    fn a_winner_is_paid_from_clearing_and_releases_margin() {
        let entries = settlement_entries("EV", "alice", 300, 500).unwrap();
        assert_eq!(total(&entries, "DR"), total(&entries, "CR"));
        assert!(entries.iter().any(|e| e.account_code == "LIAB:USER:alice:CASH"
            && e.direction == "CR"
            && e.amount_micro_usdc == 800));
        assert!(entries.iter().any(|e| e.account_code == "LIAB:HOUSE:CLEARING:EV"
            && e.direction == "DR"
            && e.amount_micro_usdc == 500));
    }

    #[test]
    fn a_flat_account_just_gets_its_margin_back() {
        let entries = settlement_entries("EV", "carol", 250, 0).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(settlement_entries("EV", "nobody", 0, 0).unwrap().is_empty());
    }

    #[test]
    fn losing_more_than_the_posted_margin_is_refused() {
        assert!(settlement_entries("EV", "bob", 100, -101).is_err());
        assert!(settlement_entries("EV", "bob", 100, -100).is_ok());
    }
}
