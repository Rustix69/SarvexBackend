//! Shadow comparison log (CM-08).
//!
//! Every pass compares, for each account that holds or works orders in two or more
//! contracts of one ISOLATED event, what cross margin would cost against what today's
//! per-order holds and per-ticker escrow cost. Nothing moves; rows go to
//! `risk.margin_shadow_log`. Gate for production, on every row for two weeks:
//!
//! ```text
//! worst_case <= capital_cross <= capital_today
//! ```
//!
//! * `capital_cross` = net premium paid on the open positions at the actual fill prices
//!   plus the cross margin `M` (positions and working orders).
//! * `capital_today` = collateral today's system keeps against the open positions
//!   (cost for a long, `max value - premium received` for a short) plus the holds on
//!   the working orders.
//! * `worst_case` = net premium plus the worst loss of the positions alone, computed
//!   through the settlement path (a different code path from `M`).
//!
//! Accounts on single-contract events are not logged: cross margin cannot differ there.

use crate::portfolio::{
    self, instrument_from_row, stored_unit_order, ContractRow, EventExposure, EventRow,
};
use crate::RiskService;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// One fill from the account's point of view, in the units me-core executed it:
/// signed YES/LONG quantity at the execution price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowFill {
    pub ticker: String,
    pub qty: i64,
    pub price_ticks: i64,
}

/// The open part of a position, by average cost. `cost_micro` carries the sign of `qty`:
/// positive premium paid for a long, negative premium received for a short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpenLot {
    pub qty: i64,
    pub cost_micro: i128,
}

impl OpenLot {
    /// Applies one fill at `unit_value` micro-USDC per unit. Reducing trades release
    /// cost pro rata; a trade through zero opens a new lot at the fill price.
    pub fn apply(&mut self, qty: i64, unit_value: i128) {
        if qty == 0 {
            return;
        }
        if self.qty == 0 || (self.qty > 0) == (qty > 0) {
            self.qty += qty;
            self.cost_micro += i128::from(qty) * unit_value;
            return;
        }
        let open = self.qty.unsigned_abs();
        let trade = qty.unsigned_abs();
        let closed = open.min(trade);
        let released = self.cost_micro * i128::from(closed) / i128::from(open);
        self.cost_micro -= released;
        self.qty += qty.signum() * i64::try_from(closed).expect("closed fits in i64");
        if trade > closed {
            let leftover = qty.signum() * i64::try_from(trade - closed).expect("fits in i64");
            self.qty = leftover;
            self.cost_micro = i128::from(leftover) * unit_value;
        }
        if self.qty == 0 {
            self.cost_micro = 0;
        }
    }

    /// What today's system keeps against this lot: the premium paid for a long, the
    /// remaining collateral `qty * max_value - premium received` for a short.
    pub fn capital_today_micro(&self, max_value_micro: i128) -> i128 {
        match self.qty {
            0 => 0,
            q if q > 0 => self.cost_micro,
            q => i128::from(q.unsigned_abs()) * max_value_micro + self.cost_micro,
        }
    }
}

/// One working order as today's system holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowOrder {
    pub ticker: String,
    pub unit_qty: i64,
    pub price_ticks: i64,
    pub hold_micro: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowRow {
    pub user_id: String,
    pub event_ticker: String,
    pub net_premium_micro: i64,
    pub margin_micro: i64,
    pub capital_cross_micro: i64,
    pub capital_today_micro: i64,
    pub worst_case_micro: i64,
    pub strategy_tag: &'static str,
    pub violation: Option<&'static str>,
}

fn narrow(value: i128) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| "shadow amount overflow".to_owned())
}

/// One account on one event. Errors say why the pair cannot be compared (the listing is
/// not eligible, a unit is unaudited, the fills do not add up to the position).
pub fn evaluate(
    user_id: &str,
    event: &EventRow,
    contracts: &BTreeMap<String, ContractRow>,
    lots: &BTreeMap<String, OpenLot>,
    positions: &BTreeMap<String, i64>,
    orders: &[ShadowOrder],
) -> Result<ShadowRow, String> {
    for (ticker, recorded) in positions {
        let derived = lots.get(ticker).map_or(0, |lot| lot.qty);
        if derived != *recorded {
            return Err(format!(
                "POSITION_MISMATCH: {ticker} fills give {derived}, positions table has {recorded}"
            ));
        }
    }
    let exposure = EventExposure {
        positions: positions
            .iter()
            .filter(|(_, qty)| **qty != 0)
            .map(|(ticker, qty)| (ticker.clone(), *qty))
            .collect(),
        orders: orders
            .iter()
            .map(|order| (order.ticker.clone(), order.unit_qty, order.price_ticks))
            .collect(),
    };
    let margin = portfolio::event_margin(event, contracts, &exposure)?;
    let worst_position_loss = portfolio::event_worst_position_loss(event, contracts, &exposure)?;

    let mut net_premium = 0_i128;
    let mut held_today = 0_i128;
    let mut kinds = BTreeSet::new();
    for (ticker, qty) in &exposure.positions {
        let row = contracts
            .get(ticker)
            .ok_or_else(|| format!("{ticker}: contract not found"))?;
        let lot = lots.get(ticker).copied().unwrap_or_default();
        debug_assert_eq!(lot.qty, *qty);
        let max_value = instrument_from_row(row)?.payoff.max_value_micro();
        net_premium += lot.cost_micro;
        held_today += lot.capital_today_micro(max_value);
        kinds.insert(row.kind.clone());
    }
    for order in orders {
        held_today += i128::from(order.hold_micro);
    }

    let capital_cross = net_premium + i128::from(margin.margin_micro);
    let worst_case = net_premium + i128::from(worst_position_loss);
    let hedged = margin.margin_micro < margin.standalone_micro;
    let strategy_tag = if exposure.positions.is_empty() {
        "ORDERS_ONLY"
    } else if exposure.positions.len() >= 2 && hedged {
        if kinds.len() >= 2 {
            "DELTA_HEDGE"
        } else if kinds.contains("BINARY") {
            "STATIC_HEDGE"
        } else {
            "MIXED"
        }
    } else if hedged {
        "MIXED"
    } else {
        "DIRECTIONAL"
    };
    let violation = if capital_cross < worst_case {
        Some("BELOW_WORST_CASE")
    } else if capital_cross > held_today {
        Some("ABOVE_TODAY")
    } else {
        None
    };
    Ok(ShadowRow {
        user_id: user_id.to_owned(),
        event_ticker: event.event_ticker.clone(),
        net_premium_micro: narrow(net_premium)?,
        margin_micro: margin.margin_micro,
        capital_cross_micro: narrow(capital_cross)?,
        capital_today_micro: narrow(held_today)?,
        worst_case_micro: narrow(worst_case)?,
        strategy_tag,
        violation,
    })
}

/// Groups skip reasons: `POSITION_MISMATCH: ...` and `LISTING_INVALID: ...` by their code,
/// `CPI-360: PAYOFF_RULE_MISSING` by what follows the contract name.
fn reason_key(reason: &str) -> String {
    match reason.split_once(": ") {
        Some((head, tail)) if head.chars().all(|c| c.is_ascii_uppercase() || c == '_') => {
            let _ = tail;
            head.to_owned()
        }
        Some((_, tail)) => tail.to_owned(),
        None => reason.to_owned(),
    }
}

/// What one pass did, for the log line.
#[derive(Debug, Default)]
pub struct PassSummary {
    pub pairs: usize,
    pub logged: usize,
    pub violations: usize,
    pub skipped: BTreeMap<String, usize>,
}

const RETENTION_DAYS: i64 = 30;

impl RiskService {
    /// One shadow pass over every account holding or working two or more contracts of a
    /// still-ISOLATED event. Bounded reads: positions once, working orders per ticker
    /// through `idx_orders_ticker_status`, fills for the involved accounts only.
    pub async fn shadow_pass(&self) -> Result<PassSummary, tonic::Status> {
        let internal = |e: sqlx::Error| tonic::Status::internal(e.to_string());
        let mut summary = PassSummary::default();

        // (user, event) -> ticker -> signed position
        let mut positions: BTreeMap<(String, String), BTreeMap<String, i64>> = BTreeMap::new();
        for row in sqlx::query(
            "SELECT p.user_id, p.ticker, c.event_ticker, p.net_qty FROM position.positions p JOIN refdata.contracts c ON c.ticker = p.ticker JOIN refdata.events e ON e.event_ticker = c.event_ticker WHERE p.net_qty <> 0 AND e.margin_model = 'ISOLATED'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?
        {
            positions
                .entry((row.get("user_id"), row.get("event_ticker")))
                .or_default()
                .insert(row.get("ticker"), row.get("net_qty"));
        }

        let tickers: Vec<(String, String)> = sqlx::query(
            "SELECT c.ticker, c.event_ticker FROM refdata.contracts c JOIN refdata.events e ON e.event_ticker = c.event_ticker WHERE e.margin_model = 'ISOLATED'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|row| (row.get("ticker"), row.get("event_ticker")))
        .collect();
        let mut orders: BTreeMap<(String, String), Vec<ShadowOrder>> = BTreeMap::new();
        for (ticker, event_ticker) in &tickers {
            let rows = sqlx::query(
                "SELECT o.user_id, o.side, o.action, o.price_ticks, GREATEST(o.count - o.filled_count - o.cancelled_count - o.expired_count, 0)::BIGINT AS remaining, COALESCE(o.hold_amount_micro_usdc, 0)::BIGINT AS hold FROM orders.orders o WHERE o.ticker = $1 AND o.status IN ('PENDING','OPEN','PARTIAL') ORDER BY o.created_at, o.order_id",
            )
            .bind(ticker)
            .fetch_all(&self.pool)
            .await
            .map_err(internal)?;
            for row in rows {
                let remaining: i64 = row.get("remaining");
                if remaining <= 0 {
                    continue;
                }
                let (unit_qty, price_ticks) = stored_unit_order(
                    row.get("side"),
                    row.get("action"),
                    row.get("price_ticks"),
                    remaining,
                );
                orders
                    .entry((row.get("user_id"), event_ticker.clone()))
                    .or_default()
                    .push(ShadowOrder {
                        ticker: ticker.clone(),
                        unit_qty,
                        price_ticks,
                        hold_micro: row.get("hold"),
                    });
            }
        }

        let mut pairs: BTreeSet<(String, String)> = positions.keys().cloned().collect();
        pairs.extend(orders.keys().cloned());
        pairs.retain(|key| {
            let mut tickers: BTreeSet<&str> = BTreeSet::new();
            if let Some(held) = positions.get(key) {
                tickers.extend(held.keys().map(String::as_str));
            }
            if let Some(working) = orders.get(key) {
                tickers.extend(working.iter().map(|o| o.ticker.as_str()));
            }
            tickers.len() >= 2
        });
        summary.pairs = pairs.len();
        if pairs.is_empty() {
            return Ok(summary);
        }

        let users: Vec<String> = pairs
            .iter()
            .map(|(user, _)| user.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let events: Vec<String> = pairs
            .iter()
            .map(|(_, event)| event.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let event_of: BTreeMap<&str, &str> = tickers
            .iter()
            .map(|(t, e)| (t.as_str(), e.as_str()))
            .collect();
        let involved: Vec<String> = tickers
            .iter()
            .filter(|(_, event)| events.contains(event))
            .map(|(ticker, _)| ticker.clone())
            .collect();

        // Open lots per (user, event, ticker), walking fills in sequence order.
        let (event_rows, contracts) = self.load_event_refdata(&events).await?;
        let mut lots: BTreeMap<(String, String), BTreeMap<String, OpenLot>> = BTreeMap::new();
        for row in sqlx::query(
            "SELECT ticker, price_ticks, count, maker_user_id, maker_side, maker_action, taker_user_id, taker_side, taker_action FROM orders.fills WHERE ticker = ANY($1) AND (maker_user_id = ANY($2) OR taker_user_id = ANY($2)) ORDER BY global_seq",
        )
        .bind(&involved)
        .bind(&users)
        .fetch_all(&self.pool)
        .await
        .map_err(internal)?
        {
            let ticker: String = row.get("ticker");
            let Some(event_ticker) = event_of.get(ticker.as_str()) else {
                continue;
            };
            let Some(contract) = contracts.get(&ticker) else {
                continue;
            };
            let Ok(instrument) = instrument_from_row(contract) else {
                continue; // reported when the pair itself is evaluated
            };
            let price: i64 = row.get("price_ticks");
            let count: i64 = row.get("count");
            let unit_value = instrument.payoff.value_of_price(price);
            for (user_col, side_col, action_col) in [
                ("maker_user_id", "maker_side", "maker_action"),
                ("taker_user_id", "taker_side", "taker_action"),
            ] {
                let user: String = row.get(user_col);
                if !users.contains(&user) {
                    continue;
                }
                let (qty, _) = stored_unit_order(row.get(side_col), row.get(action_col), price, count);
                lots.entry((user, (*event_ticker).to_owned()))
                    .or_default()
                    .entry(ticker.clone())
                    .or_default()
                    .apply(qty, unit_value);
            }
        }

        let mut rows = Vec::new();
        for key in &pairs {
            let (user, event_ticker) = key;
            let Some(event) = event_rows.get(event_ticker) else {
                *summary
                    .skipped
                    .entry("EVENT_NOT_FOUND".to_owned())
                    .or_default() += 1;
                continue;
            };
            let no_lots = BTreeMap::new();
            let no_positions = BTreeMap::new();
            let no_orders = Vec::new();
            match evaluate(
                user,
                event,
                &contracts,
                lots.get(key).unwrap_or(&no_lots),
                positions.get(key).unwrap_or(&no_positions),
                orders.get(key).unwrap_or(&no_orders),
            ) {
                Ok(row) => rows.push(row),
                Err(reason) => {
                    let reason = reason_key(&reason);
                    *summary.skipped.entry(reason).or_default() += 1;
                }
            }
        }
        summary.logged = rows.len();
        summary.violations = rows.iter().filter(|r| r.violation.is_some()).count();
        for row in &rows {
            sqlx::query(
                "INSERT INTO risk.margin_shadow_log (user_id, event_ticker, net_premium_micro_usdc, margin_micro_usdc, capital_cross_micro_usdc, capital_today_micro_usdc, worst_case_micro_usdc, strategy_tag, violation) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(&row.user_id)
            .bind(&row.event_ticker)
            .bind(row.net_premium_micro)
            .bind(row.margin_micro)
            .bind(row.capital_cross_micro)
            .bind(row.capital_today_micro)
            .bind(row.worst_case_micro)
            .bind(row.strategy_tag)
            .bind(row.violation)
            .execute(&self.pool)
            .await
            .map_err(internal)?;
        }
        sqlx::query(
            "DELETE FROM risk.margin_shadow_log WHERE as_of < now() - ($1 * interval '1 day')",
        )
        .bind(RETENTION_DAYS)
        .execute(&self.pool)
        .await
        .map_err(internal)?;
        Ok(summary)
    }
}

/// Runs a shadow pass every `interval`; a failed pass is logged and retried next time.
pub async fn run_margin_shadow_logger(service: RiskService, interval: Duration) {
    loop {
        tokio::time::sleep(interval).await;
        match service.shadow_pass().await {
            Ok(summary) => {
                tracing::info!(pairs = summary.pairs, logged = summary.logged, violations = summary.violations, skipped = ?summary.skipped, "margin shadow pass");
            }
            Err(error) => {
                tracing::warn!(error = %error.message(), "margin shadow pass failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sarvex_domain::margin::OutcomeGrid;
    use serde_json::json;

    fn event() -> EventRow {
        EventRow {
            event_ticker: "CPI".to_owned(),
            margin_model: "ISOLATED".to_owned(),
            outcome_kind: "NUMERIC".to_owned(),
            outcome_divider: Some(1),
            grid: OutcomeGrid::INTEGERS,
        }
    }

    fn binary(ticker: &str, lo: i64) -> ContractRow {
        ContractRow {
            ticker: ticker.to_owned(),
            event_ticker: "CPI".to_owned(),
            kind: "BINARY".to_owned(),
            min_price_ticks: 1,
            max_price_ticks: 99,
            lower_bound_ticks: None,
            upper_bound_ticks: None,
            tick_value_micro: 10_000,
            payoff_rule: Some(json!({"type": "range", "lo": lo, "hi": null, "outcome_divider": 1})),
            payout_ticks: 100,
            divider: 1,
            settlement_source: "BLS".to_owned(),
            expected_resolution_at: "2026-11-01".to_owned(),
            settlement_rule: Some(json!({"rounding_step": 1})),
        }
    }

    fn contracts() -> BTreeMap<String, ContractRow> {
        [binary("CPI-340", 341), binary("CPI-360", 361)]
            .into_iter()
            .map(|row| (row.ticker.clone(), row))
            .collect()
    }

    fn lot(qty: i64, price: i64) -> OpenLot {
        let mut lot = OpenLot::default();
        lot.apply(qty, i128::from(price) * 10_000);
        lot
    }

    #[test]
    fn lot_reduces_pro_rata_and_flips_through_zero() {
        let unit = |p: i128| p * 10_000;
        let mut l = OpenLot::default();
        l.apply(10, unit(40));
        l.apply(10, unit(60));
        assert_eq!((l.qty, l.cost_micro), (20, 20 * 50 * 10_000));
        l.apply(-5, unit(70));
        assert_eq!((l.qty, l.cost_micro), (15, 15 * 50 * 10_000));
        l.apply(-20, unit(30));
        assert_eq!((l.qty, l.cost_micro), (-5, -5 * 30 * 10_000));
        l.apply(5, unit(20));
        assert_eq!((l.qty, l.cost_micro), (0, 0));
    }

    #[test]
    fn collateral_of_a_short_is_max_value_less_premium_received() {
        let short = lot(-10, 40);
        assert_eq!(
            short.capital_today_micro(1_000_000),
            10 * 1_000_000 - 10 * 400_000
        );
        assert_eq!(lot(10, 40).capital_today_micro(1_000_000), 10 * 400_000);
    }

    #[test]
    fn static_hedge_is_cheaper_than_today_and_covers_the_worst_case() {
        // Long CPI>3.40 at 60, short CPI>3.60 at 25: a bull spread.
        let lots: BTreeMap<_, _> = [
            ("CPI-340".to_owned(), lot(10, 60)),
            ("CPI-360".to_owned(), lot(-10, 25)),
        ]
        .into_iter()
        .collect();
        let positions: BTreeMap<_, _> = [("CPI-340".to_owned(), 10), ("CPI-360".to_owned(), -10)]
            .into_iter()
            .collect();
        let row = evaluate("u1", &event(), &contracts(), &lots, &positions, &[]).unwrap();
        // Premium paid 10*0.60 - 10*0.25 = 3.50; worst loss of the spread is 0 above the
        // long and 10*(1.00-1.00) ... the spread can lose at most its net premium.
        assert_eq!(row.net_premium_micro, 3_500_000);
        assert_eq!(row.strategy_tag, "STATIC_HEDGE");
        assert_eq!(row.violation, None);
        assert!(row.worst_case_micro <= row.capital_cross_micro);
        assert!(row.capital_cross_micro <= row.capital_today_micro);
        // Today locks cost 6.00 for the long plus collateral 7.50 for the short.
        assert_eq!(row.capital_today_micro, 6_000_000 + 7_500_000);
        assert_eq!(row.capital_cross_micro, 3_500_000 + row.margin_micro);
    }

    #[test]
    fn working_orders_count_their_hold_on_the_today_side() {
        let lots: BTreeMap<_, _> = [("CPI-340".to_owned(), lot(10, 60))].into_iter().collect();
        let positions: BTreeMap<_, _> = [("CPI-340".to_owned(), 10)].into_iter().collect();
        let orders = [ShadowOrder {
            ticker: "CPI-360".to_owned(),
            unit_qty: -10,
            price_ticks: 25,
            hold_micro: 7_500_000,
        }];
        let row = evaluate("u1", &event(), &contracts(), &lots, &positions, &orders).unwrap();
        assert_eq!(row.capital_today_micro, 6_000_000 + 7_500_000);
        assert_eq!(row.violation, None);
    }

    #[test]
    fn a_position_the_fills_do_not_explain_is_not_logged() {
        let lots: BTreeMap<_, _> = [("CPI-340".to_owned(), lot(10, 60))].into_iter().collect();
        let positions: BTreeMap<_, _> = [("CPI-340".to_owned(), 12)].into_iter().collect();
        let error = evaluate("u1", &event(), &contracts(), &lots, &positions, &[]).unwrap_err();
        assert!(error.starts_with("POSITION_MISMATCH"), "{error}");
    }

    #[test]
    fn an_unaudited_binary_is_skipped_with_its_reason() {
        let mut rows = contracts();
        rows.get_mut("CPI-360").unwrap().payoff_rule = None;
        let lots: BTreeMap<_, _> = [
            ("CPI-340".to_owned(), lot(10, 60)),
            ("CPI-360".to_owned(), lot(-10, 25)),
        ]
        .into_iter()
        .collect();
        let positions: BTreeMap<_, _> = [("CPI-340".to_owned(), 10), ("CPI-360".to_owned(), -10)]
            .into_iter()
            .collect();
        let error = evaluate("u1", &event(), &rows, &lots, &positions, &[]).unwrap_err();
        assert!(
            error.contains("PAYOFF_RULE_MISSING") || error.contains("LISTING_INVALID"),
            "{error}"
        );
    }
}
