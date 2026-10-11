//! Cross margin (Stage 1, model C3) inside risk-svc.
//!
//! This module serves `GetAccountMargin` and `PreviewMargin` in SHADOW mode:
//! it computes what each account would lock under cross margin from today's
//! positions (`position.positions`) and working orders (`orders.orders`), next
//! to what separate per-contract margin locks. No money moves. The execution
//! path for PORTFOLIO events (`ReserveOrder`, `ReleaseOrder`, `ApplyFill`) is
//! implementation phase 3 and reads `risk.margin_positions` / `risk.margin_orders`
//! instead.
//!
//! Unit conventions (CM-05), confirmed against the code on 7 Oct 2026:
//! * `position.positions.net_qty` is a signed YES/LONG quantity: position-svc
//!   applies `signed_delta`, so a NO BUY fill makes it negative and a SHORT
//!   BUY fill makes it negative.
//! * `orders.orders.price_ticks` is stored exactly as entered, and me-core
//!   books every order on the contract's single book at that price, as a buy
//!   when `(side is YES or LONG) == (action is BUY)`. So a stored "NO BUY at 20"
//!   executes today as a YES sell at 20, not at 80. [`stored_unit_order`]
//!   mirrors that execution, because it is what the account is exposed to.
//! * The API means a NO price in NO terms ("NO at 20" = YES at 80). Prefix fix 4
//!   normalises NO to YES at `payout_ticks - p` (and SHORT to LONG at the same
//!   price) before an order is stored or routed; [`entered_unit_order`] is that
//!   mapping, used for previews of orders as the user enters them. Once fix 4
//!   ships, stored orders are all YES/LONG and the two mappings agree.
//! * position-svc's display cost basis flips NO prices to `100 - p`, which
//!   disagrees with me-core's execution today; fix 4 removes the disagreement.

use sarvex_contracts::sarvex::v1::{Action, EventMarginSummary, Side};
pub use sarvex_domain::margin::listing::parse_payoff_rule;
use sarvex_domain::margin::listing::{
    validate_portfolio_event, ListingContract, ListingEvent, ListingKind,
};
use sarvex_domain::margin::{
    EventBook, EventMargin, Instrument, MarginError, Outcome, OutcomeGrid, OutcomeKind, Payoff,
    Position, WorkingOrder,
};
use serde_json::Value;
use std::collections::BTreeMap;

/// The columns of `refdata.contracts` the engine needs.
#[derive(Debug, Clone)]
pub struct ContractRow {
    pub ticker: String,
    pub event_ticker: String,
    pub kind: String,
    pub min_price_ticks: i64,
    pub max_price_ticks: i64,
    pub lower_bound_ticks: Option<i64>,
    pub upper_bound_ticks: Option<i64>,
    pub tick_value_micro: i64,
    pub payoff_rule: Option<Value>,
    pub payout_ticks: i64,
    pub divider: i64,
    pub settlement_source: String,
    pub expected_resolution_at: String,
    pub settlement_rule: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub event_ticker: String,
    pub margin_model: String,
    pub outcome_kind: String,
    pub outcome_divider: Option<i64>,
    pub grid: OutcomeGrid,
}

/// One account's exposure on one event.
#[derive(Debug, Clone, Default)]
pub struct EventExposure {
    /// (ticker, signed YES/LONG quantity)
    pub positions: Vec<(String, i64)>,
    /// (ticker, signed unit quantity, price_ticks)
    pub orders: Vec<(String, i64, i64)>,
}

impl EventExposure {
    fn tickers(&self) -> Vec<String> {
        let mut tickers: Vec<String> = self
            .positions
            .iter()
            .map(|(t, _)| t.clone())
            .chain(self.orders.iter().map(|(t, _, _)| t.clone()))
            .collect();
        tickers.sort();
        tickers.dedup();
        tickers
    }
}

/// A stored working order as me-core executes it today: `(signed unit qty, price)`.
/// The stored price is used as is; see the module header.
pub fn stored_unit_order(side: i32, action: i32, price_ticks: i64, remaining: i64) -> (i64, i64) {
    let positive_side = side == Side::Yes as i32 || side == Side::Long as i32;
    let buy = action == Action::Buy as i32;
    let qty = if positive_side == buy {
        remaining
    } else {
        -remaining
    };
    (qty, price_ticks)
}

/// An order as the user enters it, normalised to engine units (CM-05):
///
/// | Entered | Engine unit order |
/// |---|---|
/// | Binary YES BUY at `p` / YES SELL at `p` | `+q` / `-q` at `p` |
/// | Binary NO BUY at `p` / NO SELL at `p` | `-q` / `+q` at `payout_ticks - p` |
/// | Future LONG BUY / SHORT SELL at `p` | `+q` at `p` |
/// | Future LONG SELL / SHORT BUY at `p` | `-q` at `p` |
pub fn entered_unit_order(
    binary: bool,
    payout_ticks: i64,
    side: i32,
    action: i32,
    price_ticks: i64,
    count: i64,
) -> (i64, i64) {
    let buy = action == Action::Buy as i32;
    let negative_side = side == Side::No as i32 || side == Side::Short as i32;
    let qty = if buy != negative_side { count } else { -count };
    let price = if binary && side == Side::No as i32 {
        payout_ticks - price_ticks
    } else {
        price_ticks
    };
    (qty, price)
}

pub fn outcome_kind(value: &str) -> Result<OutcomeKind, String> {
    match value {
        "NUMERIC" => Ok(OutcomeKind::Numeric),
        "CATEGORICAL" => Ok(OutcomeKind::Categorical),
        other => Err(format!("unknown outcome_kind {other}")),
    }
}

/// Builds the engine instrument for one contract, or the reason it cannot take part.
pub fn instrument_from_row(row: &ContractRow) -> Result<Instrument, String> {
    let payoff = match row.kind.as_str() {
        "SCALAR" => {
            let lower = row
                .lower_bound_ticks
                .ok_or_else(|| format!("{}: futures band is missing", row.ticker))?;
            let upper = row
                .upper_bound_ticks
                .ok_or_else(|| format!("{}: futures band is missing", row.ticker))?;
            if row.tick_value_micro <= 0 {
                return Err(format!("{}: tick_value_micro is not set", row.ticker));
            }
            Payoff::Banded {
                lower,
                upper,
                tick_value_micro: row.tick_value_micro,
            }
        }
        "BINARY" => {
            let rule = row
                .payoff_rule
                .as_ref()
                .ok_or_else(|| format!("{}: PAYOFF_RULE_MISSING", row.ticker))?;
            parse_payoff_rule(rule, row.payout_ticks)
                .map_err(|reason| format!("{}: {reason}", row.ticker))?
        }
        other => return Err(format!("{}: unsupported contract kind {other}", row.ticker)),
    };
    Ok(Instrument {
        ticker: row.ticker.clone(),
        payoff,
        min_price_ticks: row.min_price_ticks,
        max_price_ticks: row.max_price_ticks,
    })
}

pub fn outcome_label(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Numeric(value) => value.to_string(),
        Outcome::Category(name) => name.clone(),
        Outcome::OtherCategory => "OTHER".to_owned(),
    }
}

/// Engine result for one account on one event, or why it is not eligible.
/// CM-06 over every contract of the event, so a unit mismatch can never
/// produce a silently wrong shadow number.
pub fn check_listing(
    event: &EventRow,
    contracts: &BTreeMap<String, ContractRow>,
) -> Result<(), String> {
    let listing_event = ListingEvent {
        event_ticker: event.event_ticker.clone(),
        outcome_kind: outcome_kind(&event.outcome_kind)?,
        outcome_divider: event.outcome_divider,
        grid: event.grid,
    };
    let listing_contracts: Vec<ListingContract> = contracts
        .values()
        .filter(|row| row.event_ticker == event.event_ticker)
        .map(|row| ListingContract {
            ticker: row.ticker.clone(),
            kind: if row.kind == "SCALAR" {
                ListingKind::Future
            } else {
                ListingKind::Binary
            },
            divider: row.divider,
            lower_bound_ticks: row.lower_bound_ticks,
            upper_bound_ticks: row.upper_bound_ticks,
            tick_value_micro: row.tick_value_micro,
            min_price_ticks: row.min_price_ticks,
            max_price_ticks: row.max_price_ticks,
            payout_ticks: row.payout_ticks,
            payoff_rule: row.payoff_rule.clone(),
            settlement_source: row.settlement_source.clone(),
            expected_resolution_at: row.expected_resolution_at.clone(),
            settlement_rule: row.settlement_rule.clone(),
        })
        .collect();
    validate_portfolio_event(&listing_event, &listing_contracts).map_err(|violations| {
        format!(
            "LISTING_INVALID: {}",
            violations
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}

pub fn event_margin(
    event: &EventRow,
    contracts: &BTreeMap<String, ContractRow>,
    exposure: &EventExposure,
) -> Result<EventMargin, String> {
    let kind = outcome_kind(&event.outcome_kind)?;
    check_listing(event, contracts)?;
    let tickers = exposure.tickers();
    let mut instruments = Vec::with_capacity(tickers.len());
    for ticker in &tickers {
        let row = contracts
            .get(ticker)
            .ok_or_else(|| format!("{ticker}: contract not found"))?;
        if row.event_ticker != event.event_ticker {
            return Err(format!(
                "{ticker}: belongs to {}, not {}",
                row.event_ticker, event.event_ticker
            ));
        }
        instruments.push(instrument_from_row(row)?);
    }
    let grid =
        (kind == OutcomeKind::Numeric && event.grid != OutcomeGrid::INTEGERS).then_some(event.grid);
    let book = EventBook::new(kind, instruments, grid).map_err(engine_reason)?;
    let index = |ticker: &str| {
        book.instrument_index(ticker)
            .expect("instrument was just added")
    };
    let positions: Vec<Position> = exposure
        .positions
        .iter()
        .map(|(ticker, qty)| Position {
            instrument: index(ticker),
            qty: *qty,
        })
        .collect();
    let orders: Vec<WorkingOrder> = exposure
        .orders
        .iter()
        .filter(|(_, qty, _)| *qty != 0)
        .map(|(ticker, qty, price)| WorkingOrder {
            instrument: index(ticker),
            qty: *qty,
            price_ticks: *price,
        })
        .collect();
    book.event_margin(&positions, &orders)
        .map_err(engine_reason)
}

fn engine_reason(error: MarginError) -> String {
    error.to_string()
}

/// The proto summary for one event; ineligible events report the reason and zeros.
pub fn summarize(
    event: &EventRow,
    contracts: &BTreeMap<String, ContractRow>,
    exposure: &EventExposure,
) -> EventMarginSummary {
    let mut summary = EventMarginSummary {
        event_ticker: event.event_ticker.clone(),
        margin_model: event.margin_model.clone(),
        tickers: exposure.tickers(),
        ..Default::default()
    };
    match event_margin(event, contracts, exposure) {
        Ok(margin) => {
            summary.eligible = true;
            summary.margin_micro_usdc = margin.margin_micro;
            summary.position_requirement_micro_usdc = margin.position_requirement_micro;
            summary.order_margin_micro_usdc = margin.order_margin_micro();
            summary.standalone_micro_usdc = margin.standalone_micro;
            summary.worst_outcome = margin
                .worst_outcome
                .as_ref()
                .map(outcome_label)
                .unwrap_or_default();
        }
        Err(reason) => {
            summary.eligible = false;
            summary.ineligible_reason = reason;
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use sarvex_domain::margin::BINARY_TICK_VALUE_MICRO;
    use serde_json::json;

    const USDC: i64 = 1_000_000;

    fn contract(ticker: &str, kind: &str) -> ContractRow {
        ContractRow {
            ticker: ticker.into(),
            event_ticker: "XEV-CPI".into(),
            kind: kind.into(),
            min_price_ticks: 1,
            max_price_ticks: 99,
            lower_bound_ticks: Some(1),
            upper_bound_ticks: Some(99),
            tick_value_micro: 0,
            payoff_rule: None,
            payout_ticks: 100,
            divider: 100,
            settlement_source: "BLS CPI-U NSA, first release".into(),
            expected_resolution_at: "2026-10-14T12:30:00+00:00".into(),
            settlement_rule: Some(json!({"rounding_step": 1})),
        }
    }

    fn cpi_contracts(rule: Option<Value>) -> BTreeMap<String, ContractRow> {
        let future = ContractRow {
            min_price_ticks: 400,
            max_price_ticks: 600,
            lower_bound_ticks: Some(400),
            upper_bound_ticks: Some(600),
            tick_value_micro: 10 * USDC,
            ..contract("SXF-CPI", "SCALAR")
        };
        let binary = ContractRow {
            payoff_rule: rule,
            ..contract("SX-CPI-T450", "BINARY")
        };
        [future, binary]
            .into_iter()
            .map(|c| (c.ticker.clone(), c))
            .collect()
    }

    fn above_450() -> Option<Value> {
        Some(json!({"type":"range","lo":451,"hi":null,"outcome_divider":100}))
    }

    fn event() -> EventRow {
        EventRow {
            event_ticker: "XEV-CPI".into(),
            margin_model: "ISOLATED".into(),
            outcome_kind: "NUMERIC".into(),
            outcome_divider: Some(100),
            grid: OutcomeGrid::INTEGERS,
        }
    }

    #[test]
    fn stored_orders_follow_me_core_execution() {
        let yes = Side::Yes as i32;
        let no = Side::No as i32;
        let buy = Action::Buy as i32;
        let sell = Action::Sell as i32;
        assert_eq!(stored_unit_order(yes, buy, 40, 5), (5, 40));
        assert_eq!(stored_unit_order(yes, sell, 40, 5), (-5, 40));
        // me-core books a stored NO BUY at 20 as a YES sell at 20, so that is the exposure.
        assert_eq!(stored_unit_order(no, buy, 20, 5), (-5, 20));
        assert_eq!(
            stored_unit_order(Side::Long as i32, sell, 450, 5),
            (-5, 450)
        );
        assert_eq!(
            stored_unit_order(Side::Short as i32, sell, 450, 5),
            (5, 450)
        );
    }

    #[test]
    fn entered_orders_normalise_per_cm05() {
        let (yes, no, long, short) = (
            Side::Yes as i32,
            Side::No as i32,
            Side::Long as i32,
            Side::Short as i32,
        );
        let (buy, sell) = (Action::Buy as i32, Action::Sell as i32);
        let cases = [
            (true, yes, buy, 30, (7, 30)),
            (true, yes, sell, 30, (-7, 30)),
            (true, no, buy, 20, (-7, 80)),
            (true, no, sell, 20, (7, 80)),
            (false, long, buy, 450, (7, 450)),
            (false, short, sell, 450, (7, 450)),
            (false, long, sell, 450, (-7, 450)),
            (false, short, buy, 450, (-7, 450)),
        ];
        for (binary, side, action, price, expected) in cases {
            assert_eq!(
                entered_unit_order(binary, 100, side, action, price, 7),
                expected,
                "binary={binary} side={side} action={action} price={price}"
            );
        }
    }

    #[test]
    fn a_no_bid_at_20_reserves_what_a_yes_offer_at_80_does() {
        let contracts = cpi_contracts(above_450());
        let margin_for = |order: (i64, i64)| {
            event_margin(
                &event(),
                &contracts,
                &EventExposure {
                    positions: vec![],
                    orders: vec![("SX-CPI-T450".into(), order.0, order.1)],
                },
            )
            .unwrap()
            .margin_micro
        };
        let no_bid = entered_unit_order(true, 100, Side::No as i32, Action::Buy as i32, 20, 10);
        let yes_offer =
            entered_unit_order(true, 100, Side::Yes as i32, Action::Sell as i32, 80, 10);
        assert_eq!(margin_for(no_bid), margin_for(yes_offer));
        assert_eq!(margin_for(no_bid), 2 * USDC); // 10 x (1.00 - 0.80)
    }

    #[test]
    fn parses_payoff_rules() {
        assert_eq!(
            parse_payoff_rule(&json!({"type":"range","lo":341,"hi":null}), 100).unwrap(),
            Payoff::BinaryRange {
                lo: Some(341),
                hi: None,
                payout_ticks: 100,
                tick_value_micro: BINARY_TICK_VALUE_MICRO
            }
        );
        assert!(matches!(
            parse_payoff_rule(&json!({"type":"categories","yes_values":["HIKE_25"]}), 100).unwrap(),
            Payoff::BinaryCategory { .. }
        ));
        assert!(parse_payoff_rule(&json!({"type":"range","lo":"x"}), 100).is_err());
        assert!(parse_payoff_rule(&json!({"type":"strike"}), 100).is_err());
    }

    #[test]
    fn shadow_summary_for_the_cpi_hedge() {
        let contracts = cpi_contracts(above_450());
        let exposure = EventExposure {
            positions: vec![("SXF-CPI".into(), 1), ("SX-CPI-T450".into(), -500)],
            orders: vec![("SXF-CPI".into(), -1, 550)],
        };
        let summary = summarize(&event(), &contracts, &exposure);
        assert!(summary.eligible, "{}", summary.ineligible_reason);
        assert_eq!(summary.position_requirement_micro_usdc, 0);
        // Offering the held future at 5.50% only closes it, so it adds nothing
        // under either model (CM-09); only the short YES is locked separately.
        assert_eq!(summary.margin_micro_usdc, 0);
        assert_eq!(summary.standalone_micro_usdc, 500 * USDC);
        assert_eq!(
            summary.tickers,
            vec!["SX-CPI-T450".to_owned(), "SXF-CPI".to_owned()]
        );
    }

    #[test]
    fn binary_without_payoff_rule_is_ineligible() {
        let summary = summarize(
            &event(),
            &cpi_contracts(None),
            &EventExposure {
                positions: vec![("SX-CPI-T450".into(), 3)],
                orders: vec![],
            },
        );
        assert!(!summary.eligible);
        assert!(summary.ineligible_reason.contains("PAYOFF_RULE_MISSING"));
        assert_eq!(summary.margin_micro_usdc, 0);
    }

    #[test]
    fn a_unit_mismatch_makes_the_event_ineligible_instead_of_wrong() {
        let mut contracts = cpi_contracts(above_450());
        contracts.get_mut("SXF-CPI").unwrap().divider = 10;
        let summary = summarize(
            &event(),
            &contracts,
            &EventExposure {
                positions: vec![("SXF-CPI".into(), -1)],
                orders: vec![],
            },
        );
        assert!(!summary.eligible);
        assert!(summary.ineligible_reason.contains("DIVIDER_MISMATCH"));
    }

    #[test]
    fn the_event_grid_is_applied() {
        // Short 1 future against a 10 bp YES strip: R = 90 USDC on integers, 0 on the CPI grid.
        let mut contracts: BTreeMap<String, ContractRow> = cpi_contracts(None)
            .into_iter()
            .filter(|(ticker, _)| ticker == "SXF-CPI")
            .collect();
        let mut positions = vec![("SXF-CPI".to_owned(), -1)];
        for k in (410..=600).step_by(10) {
            let ticker = format!("SX-CPI-{k}");
            contracts.insert(
                ticker.clone(),
                ContractRow {
                    payoff_rule: Some(
                        json!({"type":"range","lo":k,"hi":null,"outcome_divider":100}),
                    ),
                    ..contract(&ticker, "BINARY")
                },
            );
            positions.push((ticker, 100));
        }
        let exposure = EventExposure {
            positions,
            orders: vec![],
        };
        let integers = event_margin(&event(), &contracts, &exposure).unwrap();
        assert_eq!(integers.position_requirement_micro, 90 * USDC);
        for row in contracts.values_mut() {
            row.settlement_rule = Some(json!({"rounding_step": 10}));
        }
        let grid_event = EventRow {
            grid: OutcomeGrid {
                step: 10,
                anchor: 0,
                min: None,
                max: None,
            },
            ..event()
        };
        let grid = event_margin(&grid_event, &contracts, &exposure).unwrap();
        assert_eq!(grid.position_requirement_micro, 0);
    }
}
