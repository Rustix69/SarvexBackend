//! Cross margin execution (implementation phase 3): `ReserveOrder`,
//! `ReleaseOrder` and `ApplyFill` for events whose `margin_model` is PORTFOLIO.
//!
//! risk-svc owns the margin state (`risk.margin_positions`, `risk.margin_orders`,
//! `risk.event_margin`) and moves the money through ledger-svc. Every operation
//! for one account runs inside a Postgres transaction that holds a per-user
//! advisory lock, so reserves, fills and releases for one account are serialised.
//!
//! Ledger first, then commit: the ledger transaction is posted while the risk
//! transaction is still open, with an idempotency key derived from the cause.
//! * A rejected or failed ledger post rolls the risk transaction back: nothing changed.
//! * A crash between the ledger post and the commit leaves the ledger ahead of
//!   `risk.event_margin.posted_micro_usdc`. A retry recomputes the same amounts and
//!   the ledger returns the existing transaction, so the retry just commits. If no
//!   retry comes, [`RiskService::reconcile_margin`] moves the difference back.
//!
//! A fill never needs new cash (see the design, "Working orders"): the buyer's
//! premium is paid out of margin that is already locked, and the seller's margin
//! can rise by at most the premium they just received. Funds are only checked when
//! an order is accepted.

use crate::portfolio::{self, ContractRow, EventExposure, EventRow};
use crate::{internal, valid_side_action, RiskService};
use sarvex_contracts::sarvex::v1::{
    ApplyFillRequest, ApplyFillResponse, LedgerEntry, MarginDecision, PostTransactionRequest,
    ReleaseOrderRequest, ReserveOrderRequest,
};
use sarvex_domain::margin::EventMargin;
use sqlx::{Postgres, Row, Transaction};
use std::collections::{BTreeMap, HashMap};
use tonic::Status;

pub const REJECT_INSUFFICIENT_MARGIN: &str = "INSUFFICIENT_MARGIN";
pub const REJECT_PAYOFF_RULE_MISSING: &str = "PAYOFF_RULE_MISSING";
pub const REJECT_NOT_PORTFOLIO: &str = "NOT_PORTFOLIO_EVENT";
pub const REJECT_NOT_ENABLED: &str = "PORTFOLIO_MARGIN_DISABLED";

#[derive(Debug, Clone)]
struct OrderRow {
    order_id: String,
    ticker: String,
    signed_remaining: i64,
    price_ticks: i64,
}

/// One account's margin state on one event, loaded under the per-user lock.
#[derive(Debug, Clone, Default)]
struct EventState {
    positions: BTreeMap<String, i64>,
    orders: Vec<OrderRow>,
    posted: i64,
    version: i64,
}

impl EventState {
    fn exposure(&self) -> EventExposure {
        EventExposure {
            positions: self
                .positions
                .iter()
                .filter(|(_, qty)| **qty != 0)
                .map(|(ticker, qty)| (ticker.clone(), *qty))
                .collect(),
            orders: self
                .orders
                .iter()
                .map(|o| (o.ticker.clone(), o.signed_remaining, o.price_ticks))
                .collect(),
        }
    }
}

fn zero_margin() -> EventMargin {
    EventMargin {
        margin_micro: 0,
        position_requirement_micro: 0,
        standalone_micro: 0,
        worst_outcome: None,
    }
}

fn margin_for(
    event: &EventRow,
    contracts: &BTreeMap<String, ContractRow>,
    state: &EventState,
) -> Result<EventMargin, String> {
    let exposure = state.exposure();
    if exposure.positions.is_empty() && exposure.orders.is_empty() {
        return Ok(zero_margin());
    }
    portfolio::event_margin(event, contracts, &exposure)
}

pub fn margin_account(user_id: &str, event_ticker: &str) -> String {
    format!("LIAB:USER:{user_id}:MARGIN:{event_ticker}")
}

fn cash_account(user_id: &str) -> String {
    format!("LIAB:USER:{user_id}:CASH")
}

fn entry(account: String, direction: &str, amount: i64, memo: &str) -> LedgerEntry {
    LedgerEntry {
        account_code: account,
        direction: direction.to_owned(),
        amount_micro_usdc: amount,
        memo: memo.to_owned(),
    }
}

/// CASH -> MARGIN when `delta > 0`, MARGIN -> CASH when `delta < 0`.
pub fn margin_move_entries(
    user_id: &str,
    event_ticker: &str,
    delta: i64,
    memo: &str,
) -> Vec<LedgerEntry> {
    if delta == 0 {
        return Vec::new();
    }
    let amount = delta.abs();
    let (from, to) = if delta > 0 {
        (cash_account(user_id), margin_account(user_id, event_ticker))
    } else {
        (margin_account(user_id, event_ticker), cash_account(user_id))
    };
    vec![entry(from, "DR", amount, memo), entry(to, "CR", amount, memo)]
}

/// What one party of a fill does to the ledger: its margin account moves by
/// `margin_delta` and its total (cash + margin) moves by `premium_in - premium_out`.
#[derive(Debug, Clone)]
pub struct FillParty {
    pub user_id: String,
    pub margin_delta: i64,
    pub premium_in: i64,
    pub premium_out: i64,
}

/// One balanced transaction for a fill: each account's net movement as a single
/// entry (so no account is driven negative in between), plus a CLEARING pair that
/// records the premium passing through `LIAB:HOUSE:CLEARING:{event}` and nets to zero.
pub fn fill_entries(
    event_ticker: &str,
    parties: &[FillParty],
    premium: i64,
) -> Result<Vec<LedgerEntry>, String> {
    let mut nets: BTreeMap<String, i128> = BTreeMap::new();
    for party in parties {
        let cash_delta = i128::from(party.premium_in)
            - i128::from(party.premium_out)
            - i128::from(party.margin_delta);
        *nets
            .entry(margin_account(&party.user_id, event_ticker))
            .or_default() += i128::from(party.margin_delta);
        *nets.entry(cash_account(&party.user_id)).or_default() += cash_delta;
    }
    let mut entries = Vec::new();
    let mut total: i128 = 0;
    for (account, net) in nets {
        if net == 0 {
            continue;
        }
        total += net;
        let amount = i64::try_from(net.abs()).map_err(|_| "fill amount overflow".to_owned())?;
        entries.push(entry(
            account,
            if net > 0 { "CR" } else { "DR" },
            amount,
            "fill",
        ));
    }
    if total != 0 {
        return Err(format!("fill does not net to zero ({total})"));
    }
    if premium > 0 {
        let clearing = format!("LIAB:HOUSE:CLEARING:{event_ticker}");
        entries.push(entry(clearing.clone(), "CR", premium, "fill premium in"));
        entries.push(entry(clearing, "DR", premium, "fill premium out"));
    }
    Ok(entries)
}

enum LedgerFailure {
    InsufficientFunds,
    Other(Status),
}

impl RiskService {
    async fn post_ledger(
        &self,
        key: &str,
        reason: &str,
        entries: Vec<LedgerEntry>,
    ) -> Result<String, LedgerFailure> {
        if entries.is_empty() {
            return Ok(String::new());
        }
        let result = self
            .ledger
            .clone()
            .post_transaction(PostTransactionRequest {
                idempotency_key: key.to_owned(),
                reason_code: reason.to_owned(),
                entries,
                metadata: None,
            })
            .await;
        match result {
            Ok(response) => Ok(response.into_inner().tx_id),
            Err(status)
                if status.code() == tonic::Code::FailedPrecondition
                    && status.message().contains("insufficient funds") =>
            {
                Err(LedgerFailure::InsufficientFunds)
            }
            Err(status) => Err(LedgerFailure::Other(status)),
        }
    }

    fn portfolio_enabled(&self) -> bool {
        self.portfolio_enabled
    }

    /// Event ticker of a contract, if it exists.
    async fn event_of(&self, ticker: &str) -> Result<Option<String>, Status> {
        sqlx::query_scalar("SELECT event_ticker FROM refdata.contracts WHERE ticker=$1")
            .bind(ticker)
            .fetch_optional(&self.pool)
            .await
            .map_err(internal)
    }

    async fn portfolio_refdata(
        &self,
        event_ticker: &str,
    ) -> Result<Option<(EventRow, BTreeMap<String, ContractRow>)>, Status> {
        let (mut events, contracts) = self
            .load_event_refdata(std::slice::from_ref(&event_ticker.to_owned()))
            .await?;
        Ok(events.remove(event_ticker).map(|event| (event, contracts)))
    }

    pub(crate) async fn reserve_order_impl(
        &self,
        request: ReserveOrderRequest,
    ) -> Result<MarginDecision, Status> {
        if request.user_id.trim().is_empty()
            || request.order_id.trim().is_empty()
            || request.ticker.trim().is_empty()
        {
            return Err(Status::invalid_argument(
                "user_id, order_id and ticker are required",
            ));
        }
        if request.count <= 0 || !valid_side_action(request.side, request.action) {
            return Err(Status::invalid_argument(
                "a valid side, action and positive count are required",
            ));
        }
        if !self.portfolio_enabled() {
            return Ok(decision_reject(
                REJECT_NOT_ENABLED,
                "cross margin is not enabled",
                "",
            ));
        }
        let event_ticker = self
            .event_of(&request.ticker)
            .await?
            .ok_or_else(|| Status::not_found("contract not found"))?;
        let Some((event, contracts)) = self.portfolio_refdata(&event_ticker).await? else {
            return Err(Status::not_found("event not found"));
        };
        if event.margin_model != "PORTFOLIO" {
            return Ok(decision_reject(
                REJECT_NOT_PORTFOLIO,
                "event does not use cross margin",
                &event_ticker,
            ));
        }
        let (unit_qty, unit_price) = portfolio::stored_unit_order(
            request.side,
            request.action,
            request.price_ticks,
            request.count,
        );

        let mut tx = self.pool.begin().await.map_err(internal)?;
        lock_users(&mut tx, &[request.user_id.as_str()]).await?;
        let mut state = load_state(&mut tx, &request.user_id, &event_ticker).await?;
        if state.orders.iter().any(|o| o.order_id == request.order_id) {
            // A retry of an order that is already reserved.
            tx.rollback().await.map_err(internal)?;
            return Ok(MarginDecision {
                approved: true,
                event_ticker,
                margin_before_micro_usdc: state.posted,
                margin_after_micro_usdc: state.posted,
                ..Default::default()
            });
        }
        let before = state.posted;
        state.orders.push(OrderRow {
            order_id: request.order_id.clone(),
            ticker: request.ticker.clone(),
            signed_remaining: unit_qty,
            price_ticks: unit_price,
        });
        let after = match margin_for(&event, &contracts, &state) {
            Ok(margin) => margin,
            Err(reason) => {
                tx.rollback().await.map_err(internal)?;
                let code = if reason.contains("PAYOFF_RULE_MISSING") {
                    REJECT_PAYOFF_RULE_MISSING
                } else {
                    "MARGIN_INELIGIBLE"
                };
                return Ok(decision_reject(code, &reason, &event_ticker));
            }
        };
        let delta = after.margin_micro - before;
        let ledger_tx = match self
            .post_ledger(
                &format!("margin:reserve:{}", request.order_id),
                "MARGIN_RESERVE",
                margin_move_entries(&request.user_id, &event_ticker, delta, "reserve order"),
            )
            .await
        {
            Ok(tx_id) => tx_id,
            Err(LedgerFailure::InsufficientFunds) => {
                tx.rollback().await.map_err(internal)?;
                return Ok(MarginDecision {
                    approved: false,
                    reject_code: REJECT_INSUFFICIENT_MARGIN.to_owned(),
                    reject_reason: "not enough cash for the margin this order needs".to_owned(),
                    event_ticker,
                    margin_before_micro_usdc: before,
                    margin_after_micro_usdc: after.margin_micro,
                    ledger_tx_id: String::new(),
                });
            }
            Err(LedgerFailure::Other(status)) => return Err(status),
        };
        sqlx::query("INSERT INTO risk.margin_orders (order_id, user_id, ticker, event_ticker, signed_remaining, price_ticks) VALUES ($1,$2,$3,$4,$5,$6)")
            .bind(&request.order_id)
            .bind(&request.user_id)
            .bind(&request.ticker)
            .bind(&event_ticker)
            .bind(unit_qty)
            .bind(unit_price)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        let version = save_event_margin(&mut tx, &request.user_id, &event_ticker, &after).await?;
        journal(
            &mut tx,
            &request.user_id,
            &event_ticker,
            "RESERVE",
            &request.order_id,
            before,
            after.margin_micro,
            &format!("margin:reserve:{}:{version}", request.order_id),
            &ledger_tx,
        )
        .await?;
        tx.commit().await.map_err(internal)?;
        Ok(MarginDecision {
            approved: true,
            event_ticker,
            margin_before_micro_usdc: before,
            margin_after_micro_usdc: after.margin_micro,
            ledger_tx_id: ledger_tx,
            ..Default::default()
        })
    }

    pub(crate) async fn release_order_impl(
        &self,
        request: ReleaseOrderRequest,
    ) -> Result<MarginDecision, Status> {
        if request.user_id.trim().is_empty() || request.order_id.trim().is_empty() {
            return Err(Status::invalid_argument(
                "user_id and order_id are required",
            ));
        }
        if request.remaining_count < 0 {
            return Err(Status::invalid_argument(
                "remaining_count cannot be negative",
            ));
        }
        let mut tx = self.pool.begin().await.map_err(internal)?;
        lock_users(&mut tx, &[request.user_id.as_str()]).await?;
        let row = sqlx::query("SELECT event_ticker, signed_remaining, last_global_seq FROM risk.margin_orders WHERE order_id=$1 AND user_id=$2 FOR UPDATE")
            .bind(&request.order_id)
            .bind(&request.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(internal)?;
        let Some(row) = row else {
            // Already released: a replay.
            tx.rollback().await.map_err(internal)?;
            return Ok(MarginDecision {
                approved: true,
                ..Default::default()
            });
        };
        let event_ticker: String = row.get("event_ticker");
        let signed_remaining: i64 = row.get("signed_remaining");
        let last_seq: i64 = row.get("last_global_seq");
        if (request.global_seq as i64) < last_seq {
            tx.rollback().await.map_err(internal)?;
            return Ok(decision_reject(
                "STALE_RELEASE",
                "release is older than the last applied event for this order",
                &event_ticker,
            ));
        }
        let Some((event, contracts)) = self.portfolio_refdata(&event_ticker).await? else {
            return Err(Status::not_found("event not found"));
        };
        let mut state = load_state(&mut tx, &request.user_id, &event_ticker).await?;
        let before = state.posted;
        let keep = request.remaining_count.min(signed_remaining.abs());
        if keep == 0 {
            state.orders.retain(|o| o.order_id != request.order_id);
        } else if let Some(order) = state
            .orders
            .iter_mut()
            .find(|o| o.order_id == request.order_id)
        {
            order.signed_remaining = keep * signed_remaining.signum();
        }
        let after = margin_for(&event, &contracts, &state).map_err(Status::failed_precondition)?;
        let delta = after.margin_micro - before;
        let ledger_tx = match self
            .post_ledger(
                &format!("margin:release:{}:{}", request.order_id, state.version),
                "MARGIN_RELEASE",
                margin_move_entries(&request.user_id, &event_ticker, delta, "release order"),
            )
            .await
        {
            Ok(tx_id) => tx_id,
            Err(LedgerFailure::InsufficientFunds) => {
                // Releasing can only move money back to cash, so this means the
                // ledger and risk state disagree; the reconciler reports it.
                return Err(Status::failed_precondition(
                    "margin account is below the amount to release",
                ));
            }
            Err(LedgerFailure::Other(status)) => return Err(status),
        };
        if keep == 0 {
            sqlx::query("DELETE FROM risk.margin_orders WHERE order_id=$1")
                .bind(&request.order_id)
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
        } else {
            sqlx::query("UPDATE risk.margin_orders SET signed_remaining=$2, updated_at=now() WHERE order_id=$1")
                .bind(&request.order_id)
                .bind(keep * signed_remaining.signum())
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
        }
        let version = save_event_margin(&mut tx, &request.user_id, &event_ticker, &after).await?;
        journal(
            &mut tx,
            &request.user_id,
            &event_ticker,
            "RELEASE",
            &request.order_id,
            before,
            after.margin_micro,
            &format!("margin:release:{}:{version}", request.order_id),
            &ledger_tx,
        )
        .await?;
        tx.commit().await.map_err(internal)?;
        Ok(MarginDecision {
            approved: true,
            event_ticker,
            margin_before_micro_usdc: before,
            margin_after_micro_usdc: after.margin_micro,
            ledger_tx_id: ledger_tx,
            ..Default::default()
        })
    }

    pub(crate) async fn apply_fill_impl(
        &self,
        request: ApplyFillRequest,
    ) -> Result<ApplyFillResponse, Status> {
        if request.fill_id.trim().is_empty()
            || request.ticker.trim().is_empty()
            || request.maker_user_id.trim().is_empty()
            || request.taker_user_id.trim().is_empty()
        {
            return Err(Status::invalid_argument(
                "fill_id, ticker and both user ids are required",
            ));
        }
        if request.count <= 0 || request.price_ticks <= 0 {
            return Err(Status::invalid_argument(
                "fill price and count must be positive",
            ));
        }
        if !valid_side_action(request.maker_side, request.maker_action)
            || !valid_side_action(request.taker_side, request.taker_action)
        {
            return Err(Status::invalid_argument("invalid side or action"));
        }
        let event_ticker = self
            .event_of(&request.ticker)
            .await?
            .ok_or_else(|| Status::not_found("contract not found"))?;
        let Some((event, contracts)) = self.portfolio_refdata(&event_ticker).await? else {
            return Err(Status::not_found("event not found"));
        };
        if event.margin_model != "PORTFOLIO" {
            return Err(Status::failed_precondition(
                "ApplyFill is only valid for PORTFOLIO events",
            ));
        }
        let contract = contracts
            .get(&request.ticker)
            .ok_or_else(|| Status::not_found("contract not found"))?;
        let (maker_qty, price) = portfolio::stored_unit_order(
            request.maker_side,
            request.maker_action,
            request.price_ticks,
            request.count,
        );
        let (taker_qty, _) = portfolio::stored_unit_order(
            request.taker_side,
            request.taker_action,
            request.price_ticks,
            request.count,
        );
        if maker_qty != -taker_qty {
            return Err(Status::invalid_argument(
                "maker and taker must be on opposite sides of the YES/LONG book",
            ));
        }
        let premium = portfolio::trade_premium_micro(contract, price, request.count)
            .map_err(Status::failed_precondition)?;

        let mut users: Vec<&str> = vec![&request.maker_user_id, &request.taker_user_id];
        users.sort_unstable();
        users.dedup();
        let mut tx = self.pool.begin().await.map_err(internal)?;
        lock_users(&mut tx, &users).await?;
        let inserted = sqlx::query("INSERT INTO risk.applied_fills (fill_id, event_ticker, global_seq) VALUES ($1,$2,$3) ON CONFLICT (fill_id) DO NOTHING")
            .bind(&request.fill_id)
            .bind(&event_ticker)
            .bind(request.global_seq as i64)
            .execute(&mut *tx)
            .await
            .map_err(internal)?
            .rows_affected();
        if inserted == 0 {
            let maker = load_state(&mut tx, &request.maker_user_id, &event_ticker).await?;
            let taker = load_state(&mut tx, &request.taker_user_id, &event_ticker).await?;
            tx.rollback().await.map_err(internal)?;
            return Ok(ApplyFillResponse {
                applied: false,
                duplicate: true,
                ledger_tx_id: String::new(),
                maker_margin_after_micro_usdc: maker.posted,
                taker_margin_after_micro_usdc: taker.posted,
            });
        }

        let mut states: HashMap<String, EventState> = HashMap::new();
        for user in &users {
            states.insert(
                (*user).to_owned(),
                load_state(&mut tx, user, &event_ticker).await?,
            );
        }
        let before: HashMap<String, i64> = states
            .iter()
            .map(|(user, state)| (user.clone(), state.posted))
            .collect();
        // The side of the fill that gains units pays the premium.
        let legs = [
            (&request.maker_user_id, &request.maker_order_id, maker_qty),
            (&request.taker_user_id, &request.taker_order_id, taker_qty),
        ];
        for (user, order_id, qty) in legs {
            let state = states.get_mut(user).expect("state loaded for both users");
            *state.positions.entry(request.ticker.clone()).or_default() += qty;
            if let Some(index) = state.orders.iter().position(|o| &o.order_id == order_id) {
                let order = &mut state.orders[index];
                let remaining = order.signed_remaining.abs() - request.count;
                if remaining <= 0 {
                    state.orders.remove(index);
                } else {
                    order.signed_remaining = remaining * order.signed_remaining.signum();
                }
            }
        }
        let mut margins: HashMap<String, EventMargin> = HashMap::new();
        for (user, state) in &states {
            let margin = margin_for(&event, &contracts, state).map_err(|reason| {
                Status::failed_precondition(format!("margin for {user} after fill: {reason}"))
            })?;
            margins.insert(user.clone(), margin);
        }
        let mut parties: Vec<FillParty> = Vec::new();
        for user in &users {
            let mut party = FillParty {
                user_id: (*user).to_owned(),
                margin_delta: margins[*user].margin_micro - before[*user],
                premium_in: 0,
                premium_out: 0,
            };
            for (leg_user, _, qty) in legs {
                if leg_user.as_str() == *user {
                    if qty > 0 {
                        party.premium_out += premium;
                    } else {
                        party.premium_in += premium;
                    }
                }
            }
            parties.push(party);
        }
        let entries = fill_entries(&event_ticker, &parties, premium)
            .map_err(Status::failed_precondition)?;
        let ledger_tx = match self
            .post_ledger(
                &format!("margin:fill:{}", request.fill_id),
                "MARGIN_FILL",
                entries,
            )
            .await
        {
            Ok(tx_id) => tx_id,
            Err(LedgerFailure::InsufficientFunds) => {
                return Err(Status::failed_precondition(
                    "fill would take an account below zero; the margin proof was violated",
                ));
            }
            Err(LedgerFailure::Other(status)) => return Err(status),
        };
        for (user, _, qty) in legs {
            sqlx::query("INSERT INTO risk.margin_positions (user_id, ticker, event_ticker, net_qty, last_global_seq) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (user_id, ticker) DO UPDATE SET net_qty = risk.margin_positions.net_qty + EXCLUDED.net_qty, last_global_seq = GREATEST(risk.margin_positions.last_global_seq, EXCLUDED.last_global_seq), updated_at = now()")
                .bind(user)
                .bind(&request.ticker)
                .bind(&event_ticker)
                .bind(qty)
                .bind(request.global_seq as i64)
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
        }
        for (user, order_id, _) in legs {
            let state = &states[user];
            match state.orders.iter().find(|o| &o.order_id == order_id) {
                Some(order) => {
                    sqlx::query("UPDATE risk.margin_orders SET signed_remaining=$2, last_global_seq=GREATEST(last_global_seq,$3), updated_at=now() WHERE order_id=$1")
                        .bind(order_id)
                        .bind(order.signed_remaining)
                        .bind(request.global_seq as i64)
                        .execute(&mut *tx)
                        .await
                        .map_err(internal)?;
                }
                None => {
                    sqlx::query("DELETE FROM risk.margin_orders WHERE order_id=$1")
                        .bind(order_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(internal)?;
                }
            }
        }
        sqlx::query("UPDATE risk.applied_fills SET ledger_tx_id=NULLIF($2,'') WHERE fill_id=$1")
            .bind(&request.fill_id)
            .bind(&ledger_tx)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        for user in &users {
            let version =
                save_event_margin(&mut tx, user, &event_ticker, &margins[*user]).await?;
            journal(
                &mut tx,
                user,
                &event_ticker,
                "FILL",
                &request.fill_id,
                before[*user],
                margins[*user].margin_micro,
                &format!("margin:fill:{}:{user}:{version}", request.fill_id),
                &ledger_tx,
            )
            .await?;
        }
        tx.commit().await.map_err(internal)?;
        Ok(ApplyFillResponse {
            applied: true,
            duplicate: false,
            ledger_tx_id: ledger_tx,
            maker_margin_after_micro_usdc: margins[&request.maker_user_id].margin_micro,
            taker_margin_after_micro_usdc: margins[&request.taker_user_id].margin_micro,
        })
    }

    /// Margin and orders of one account on PORTFOLIO events, for `GetAccountMargin`
    /// and `PreviewMargin`: these tables, not `position.positions`, are authoritative.
    pub(crate) async fn load_margin_exposures(
        &self,
        user_id: &str,
        event_ticker: Option<&str>,
    ) -> Result<BTreeMap<String, EventExposure>, Status> {
        let mut exposures: BTreeMap<String, EventExposure> = BTreeMap::new();
        let positions = sqlx::query("SELECT ticker, event_ticker, net_qty FROM risk.margin_positions WHERE user_id=$1 AND net_qty<>0 AND ($2::TEXT IS NULL OR event_ticker=$2) ORDER BY ticker")
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
        let orders = sqlx::query("SELECT ticker, event_ticker, signed_remaining, price_ticks FROM risk.margin_orders WHERE user_id=$1 AND ($2::TEXT IS NULL OR event_ticker=$2) ORDER BY created_at, order_id")
            .bind(user_id)
            .bind(event_ticker)
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        for row in orders {
            exposures
                .entry(row.get("event_ticker"))
                .or_default()
                .orders
                .push((
                    row.get("ticker"),
                    row.get("signed_remaining"),
                    row.get("price_ticks"),
                ));
        }
        Ok(exposures)
    }

    /// Compares every `MARGIN` ledger balance with `risk.event_margin.posted_micro_usdc`.
    /// A mismatch seen on two consecutive passes (so in-flight operations are not touched)
    /// is corrected by moving the difference between CASH and MARGIN. Returns what it found.
    pub async fn reconcile_margin(
        &self,
        suspects: &mut HashMap<(String, String), i64>,
    ) -> Result<Vec<MarginDrift>, Status> {
        let rows = sqlx::query("SELECT em.user_id, em.event_ticker, em.posted_micro_usdc, COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code = 'LIAB:USER:' || em.user_id || ':MARGIN:' || em.event_ticker ORDER BY e.entry_id DESC LIMIT 1), 0)::BIGINT AS ledger_balance FROM risk.event_margin em")
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        let mut found = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for row in rows {
            let user_id: String = row.get("user_id");
            let event_ticker: String = row.get("event_ticker");
            let posted: i64 = row.get("posted_micro_usdc");
            let balance: i64 = row.get("ledger_balance");
            let key = (user_id.clone(), event_ticker.clone());
            if posted == balance {
                continue;
            }
            seen.insert(key.clone());
            let drift = MarginDrift {
                user_id: user_id.clone(),
                event_ticker: event_ticker.clone(),
                posted,
                ledger_balance: balance,
            };
            if suspects.get(&key) != Some(&balance) {
                suspects.insert(key, balance);
                found.push(drift);
                continue;
            }
            // Second sighting: correct it under the user's lock, from fresh state.
            let mut tx = self.pool.begin().await.map_err(internal)?;
            lock_users(&mut tx, &[user_id.as_str()]).await?;
            let state = load_state(&mut tx, &user_id, &event_ticker).await?;
            let current: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code=$1 ORDER BY e.entry_id DESC LIMIT 1),0)::BIGINT")
                .bind(margin_account(&user_id, &event_ticker))
                .fetch_one(&mut *tx)
                .await
                .map_err(internal)?;
            let diff = state.posted - current;
            if diff != 0 {
                let result = self
                    .post_ledger(
                        &format!(
                            "margin:reconcile:{user_id}:{event_ticker}:{}:{current}",
                            state.version
                        ),
                        "MARGIN_RECONCILE",
                        margin_move_entries(&user_id, &event_ticker, diff, "reconcile"),
                    )
                    .await;
                match result {
                    Ok(ledger_tx) => {
                        journal(
                            &mut tx,
                            &user_id,
                            &event_ticker,
                            "RECONCILE",
                            &format!("{current}->{}", state.posted),
                            current,
                            state.posted,
                            &format!(
                                "margin:reconcile:{user_id}:{event_ticker}:{}:{current}",
                                state.version
                            ),
                            &ledger_tx,
                        )
                        .await?;
                        tx.commit().await.map_err(internal)?;
                        tracing::warn!(user_id, event_ticker, posted = state.posted, ledger = current, "margin drift corrected");
                    }
                    Err(LedgerFailure::InsufficientFunds) => {
                        tx.rollback().await.map_err(internal)?;
                        tracing::error!(user_id, event_ticker, posted = state.posted, ledger = current, "margin drift cannot be corrected: not enough cash");
                    }
                    Err(LedgerFailure::Other(status)) => {
                        tx.rollback().await.map_err(internal)?;
                        tracing::error!(user_id, event_ticker, error = %status, "margin drift correction failed");
                    }
                }
            } else {
                tx.rollback().await.map_err(internal)?;
            }
            found.push(drift);
            suspects.remove(&(user_id, event_ticker));
        }
        suspects.retain(|key, _| seen.contains(key));
        Ok(found)
    }

    /// The ledger invariants of the design, as human-readable violations.
    pub async fn margin_invariants(&self) -> Result<Vec<String>, Status> {
        let mut violations = Vec::new();
        let rows = sqlx::query("SELECT a.account_code, COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e WHERE e.account_id=a.account_id ORDER BY e.entry_id DESC LIMIT 1),0)::BIGINT AS balance FROM ledger.accounts a WHERE a.account_code LIKE 'LIAB:HOUSE:CLEARING:%' OR a.account_code LIKE 'LIAB:USER:%:MARGIN:%' OR a.account_code LIKE 'LIAB:USER:%:CASH' OR a.account_code LIKE 'LIAB:USER:%:HOLDS'")
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        for row in rows {
            let code: String = row.get("account_code");
            let balance: i64 = row.get("balance");
            if code.starts_with("LIAB:HOUSE:CLEARING:") && balance != 0 {
                violations.push(format!("{code} should be 0 but is {balance}"));
            }
            if code.starts_with("LIAB:USER:") && balance < 0 {
                violations.push(format!("{code} is negative ({balance})"));
            }
        }
        let drift = sqlx::query("SELECT em.user_id, em.event_ticker, em.posted_micro_usdc, COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code = 'LIAB:USER:' || em.user_id || ':MARGIN:' || em.event_ticker ORDER BY e.entry_id DESC LIMIT 1), 0)::BIGINT AS ledger_balance FROM risk.event_margin em WHERE em.posted_micro_usdc <> COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code = 'LIAB:USER:' || em.user_id || ':MARGIN:' || em.event_ticker ORDER BY e.entry_id DESC LIMIT 1), 0)")
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        for row in drift {
            violations.push(format!(
                "MARGIN:{}:{} ledger {} != posted {}",
                row.get::<String, _>("user_id"),
                row.get::<String, _>("event_ticker"),
                row.get::<i64, _>("ledger_balance"),
                row.get::<i64, _>("posted_micro_usdc"),
            ));
        }
        let open_interest = sqlx::query("SELECT ticker, SUM(net_qty)::BIGINT AS net FROM risk.margin_positions GROUP BY ticker HAVING SUM(net_qty) <> 0")
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
        for row in open_interest {
            violations.push(format!(
                "open interest of {} does not net to zero ({})",
                row.get::<String, _>("ticker"),
                row.get::<i64, _>("net")
            ));
        }
        Ok(violations)
    }
}

#[derive(Debug, Clone)]
pub struct MarginDrift {
    pub user_id: String,
    pub event_ticker: String,
    pub posted: i64,
    pub ledger_balance: i64,
}

fn decision_reject(code: &str, reason: &str, event_ticker: &str) -> MarginDecision {
    MarginDecision {
        approved: false,
        reject_code: code.to_owned(),
        reject_reason: reason.to_owned(),
        event_ticker: event_ticker.to_owned(),
        ..Default::default()
    }
}

/// Serialises everything for these accounts. Users are locked in sorted order
/// so two fills touching the same pair can never deadlock.
async fn lock_users(tx: &mut Transaction<'_, Postgres>, users: &[&str]) -> Result<(), Status> {
    let mut sorted: Vec<&str> = users.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for user in sorted {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("margin:{user}"))
            .execute(&mut **tx)
            .await
            .map_err(internal)?;
    }
    Ok(())
}

async fn load_state(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    event_ticker: &str,
) -> Result<EventState, Status> {
    let mut state = EventState::default();
    for row in sqlx::query("SELECT ticker, net_qty FROM risk.margin_positions WHERE user_id=$1 AND event_ticker=$2 ORDER BY ticker")
        .bind(user_id)
        .bind(event_ticker)
        .fetch_all(&mut **tx)
        .await
        .map_err(internal)?
    {
        state.positions.insert(row.get("ticker"), row.get("net_qty"));
    }
    for row in sqlx::query("SELECT order_id, ticker, signed_remaining, price_ticks FROM risk.margin_orders WHERE user_id=$1 AND event_ticker=$2 ORDER BY created_at, order_id")
        .bind(user_id)
        .bind(event_ticker)
        .fetch_all(&mut **tx)
        .await
        .map_err(internal)?
    {
        state.orders.push(OrderRow {
            order_id: row.get("order_id"),
            ticker: row.get("ticker"),
            signed_remaining: row.get("signed_remaining"),
            price_ticks: row.get("price_ticks"),
        });
    }
    if let Some(row) = sqlx::query("SELECT posted_micro_usdc, version FROM risk.event_margin WHERE user_id=$1 AND event_ticker=$2")
        .bind(user_id)
        .bind(event_ticker)
        .fetch_optional(&mut **tx)
        .await
        .map_err(internal)?
    {
        state.posted = row.get("posted_micro_usdc");
        state.version = row.get("version");
    }
    Ok(state)
}

async fn save_event_margin(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    event_ticker: &str,
    margin: &EventMargin,
) -> Result<i64, Status> {
    let worst = margin.worst_outcome.as_ref().map(portfolio::outcome_label);
    sqlx::query_scalar("INSERT INTO risk.event_margin (user_id, event_ticker, margin_micro_usdc, position_requirement_micro_usdc, posted_micro_usdc, standalone_micro_usdc, worst_outcome, version) VALUES ($1,$2,$3,$4,$3,$5,$6,1) ON CONFLICT (user_id, event_ticker) DO UPDATE SET margin_micro_usdc=EXCLUDED.margin_micro_usdc, position_requirement_micro_usdc=EXCLUDED.position_requirement_micro_usdc, posted_micro_usdc=EXCLUDED.posted_micro_usdc, standalone_micro_usdc=EXCLUDED.standalone_micro_usdc, worst_outcome=EXCLUDED.worst_outcome, version=risk.event_margin.version+1, updated_at=now() RETURNING version")
        .bind(user_id)
        .bind(event_ticker)
        .bind(margin.margin_micro)
        .bind(margin.position_requirement_micro)
        .bind(margin.standalone_micro)
        .bind(worst)
        .fetch_one(&mut **tx)
        .await
        .map_err(internal)
}

#[allow(clippy::too_many_arguments)]
async fn journal(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    event_ticker: &str,
    cause: &str,
    cause_id: &str,
    before: i64,
    after: i64,
    key: &str,
    ledger_tx_id: &str,
) -> Result<(), Status> {
    sqlx::query("INSERT INTO risk.margin_journal (user_id, event_ticker, cause, cause_id, before_micro_usdc, after_micro_usdc, ledger_idempotency_key, ledger_tx_id, status, posted_at) VALUES ($1,$2,$3,$4,$5,$6,$7,NULLIF($8,''),'POSTED',now()) ON CONFLICT (ledger_idempotency_key) DO NOTHING")
        .bind(user_id)
        .bind(event_ticker)
        .bind(cause)
        .bind(cause_id)
        .bind(before)
        .bind(after)
        .bind(key)
        .bind(ledger_tx_id)
        .execute(&mut **tx)
        .await
        .map_err(internal)?;
    Ok(())
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
    fn margin_moves_go_the_right_way() {
        let up = margin_move_entries("u1", "EV", 500, "x");
        assert_eq!(up[0].account_code, "LIAB:USER:u1:CASH");
        assert_eq!(up[0].direction, "DR");
        assert_eq!(up[1].account_code, "LIAB:USER:u1:MARGIN:EV");
        let down = margin_move_entries("u1", "EV", -500, "x");
        assert_eq!(down[0].account_code, "LIAB:USER:u1:MARGIN:EV");
        assert_eq!(down[0].direction, "DR");
        assert!(margin_move_entries("u1", "EV", 0, "x").is_empty());
    }

    #[test]
    fn fill_entries_balance_and_clear() {
        // Buyer pays 400 of premium out of margin and releases 100; seller receives 400
        // and has to post 250 more.
        let parties = vec![
            FillParty {
                user_id: "buyer".into(),
                margin_delta: -500,
                premium_in: 0,
                premium_out: 400,
            },
            FillParty {
                user_id: "seller".into(),
                margin_delta: 250,
                premium_in: 400,
                premium_out: 0,
            },
        ];
        let entries = fill_entries("EV", &parties, 400).unwrap();
        assert_eq!(total(&entries, "DR"), total(&entries, "CR"));
        let cash = |user: &str| {
            entries
                .iter()
                .find(|e| e.account_code == format!("LIAB:USER:{user}:CASH"))
                .map(|e| (e.direction.clone(), e.amount_micro_usdc))
        };
        // buyer: margin 500 -> 0 net; 400 leaves as premium, 100 returns to cash
        assert_eq!(cash("buyer"), Some(("CR".into(), 100)));
        // seller: +400 premium, 250 into margin -> cash +150
        assert_eq!(cash("seller"), Some(("CR".into(), 150)));
        let clearing: i64 = entries
            .iter()
            .filter(|e| e.account_code.starts_with("LIAB:HOUSE:CLEARING:"))
            .map(|e| if e.direction == "CR" { e.amount_micro_usdc } else { -e.amount_micro_usdc })
            .sum();
        assert_eq!(clearing, 0);
    }

    #[test]
    fn self_trade_nets_to_nothing_but_the_margin_change() {
        let parties = vec![FillParty {
            user_id: "u".into(),
            margin_delta: -30,
            premium_in: 400,
            premium_out: 400,
        }];
        let entries = fill_entries("EV", &parties, 400).unwrap();
        assert_eq!(total(&entries, "DR"), total(&entries, "CR"));
        // margin account -30, cash +30
        assert!(entries
            .iter()
            .any(|e| e.account_code == "LIAB:USER:u:MARGIN:EV" && e.direction == "DR" && e.amount_micro_usdc == 30));
    }

    #[test]
    fn a_fill_with_no_money_movement_posts_nothing() {
        let parties = vec![
            FillParty { user_id: "a".into(), margin_delta: 0, premium_in: 0, premium_out: 0 },
            FillParty { user_id: "b".into(), margin_delta: 0, premium_in: 0, premium_out: 0 },
        ];
        assert!(fill_entries("EV", &parties, 0).unwrap().is_empty());
    }
}
