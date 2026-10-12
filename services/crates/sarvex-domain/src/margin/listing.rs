//! Listing rules for PORTFOLIO events (CM-06) and the shared `payoff_rule` parser.
//!
//! The engine assumes every contract on an event measures the outcome `x` on
//! one axis, in the same ticks, resolved once from one source. It cannot detect
//! a binary strike written in tenths of a percent next to a futures band in
//! basis points; it would silently compute a wrong worst case. These checks
//! run before a PORTFOLIO event may open (refdata-svc `TransitionState`) and
//! before risk-svc trusts a shadow-mode number.
//!
//! Different trading close times are allowed: the math only needs one resolved
//! value and one settlement pass. A contract that stops trading early simply has
//! its orders expired.

use super::{Instrument, OutcomeGrid, OutcomeKind, Payoff, BINARY_TICK_VALUE_MICRO};
use serde_json::Value;
use std::fmt;

/// Parses `refdata.contracts.payoff_rule` for a binary. Shared by risk-svc
/// (margin) and settlement-svc (resolution), so both read strikes the same way.
///
/// * `{"type":"range","lo":341,"hi":null,"outcome_divider":100}`: YES iff `lo <= x <= hi`
/// * `{"type":"categories","yes_values":["HIKE_25"]}`: YES iff the category is listed
pub fn parse_payoff_rule(rule: &Value, payout_ticks: i64) -> Result<Payoff, String> {
    let bound = |name: &str| -> Result<Option<i64>, String> {
        match rule.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_i64()
                .map(Some)
                .ok_or_else(|| format!("payoff_rule.{name} must be an integer or null")),
        }
    };
    match rule.get("type").and_then(Value::as_str) {
        Some("range") => Ok(Payoff::BinaryRange {
            lo: bound("lo")?,
            hi: bound("hi")?,
            payout_ticks,
            tick_value_micro: BINARY_TICK_VALUE_MICRO,
        }),
        Some("categories") => {
            let values = rule
                .get("yes_values")
                .and_then(Value::as_array)
                .ok_or("payoff_rule.yes_values must be an array")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or("yes_values must be strings")
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Payoff::BinaryCategory {
                yes_values: values,
                payout_ticks,
                tick_value_micro: BINARY_TICK_VALUE_MICRO,
            })
        }
        _ => Err("payoff_rule.type must be \"range\" or \"categories\"".to_owned()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingKind {
    Binary,
    Future,
}

/// What listing validation needs from `refdata.events`.
#[derive(Debug, Clone)]
pub struct ListingEvent {
    pub event_ticker: String,
    pub outcome_kind: OutcomeKind,
    /// Raw outcome ticks per display unit (`refdata.events.outcome_divider`).
    pub outcome_divider: Option<i64>,
    pub grid: OutcomeGrid,
}

/// What listing validation needs from one `refdata.contracts` row.
#[derive(Debug, Clone)]
pub struct ListingContract {
    pub ticker: String,
    pub kind: ListingKind,
    pub divider: i64,
    pub lower_bound_ticks: Option<i64>,
    pub upper_bound_ticks: Option<i64>,
    pub tick_value_micro: i64,
    pub min_price_ticks: i64,
    pub max_price_ticks: i64,
    pub payout_ticks: i64,
    pub payoff_rule: Option<Value>,
    pub settlement_source: String,
    /// `expected_resolution_at` as text (RFC 3339) or epoch seconds; compared for equality.
    pub expected_resolution_at: String,
    pub settlement_rule: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub code: &'static str,
    pub ticker: Option<String>,
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.ticker {
            Some(ticker) => write!(f, "{} {}: {}", self.code, ticker, self.detail),
            None => write!(f, "{}: {}", self.code, self.detail),
        }
    }
}

fn violation(code: &'static str, ticker: Option<&str>, detail: impl Into<String>) -> Violation {
    Violation {
        code,
        ticker: ticker.map(str::to_owned),
        detail: detail.into(),
    }
}

/// Builds the engine instrument for one contract row.
pub fn instrument(contract: &ListingContract) -> Result<Instrument, String> {
    let payoff = match contract.kind {
        ListingKind::Future => {
            let (Some(lower), Some(upper)) =
                (contract.lower_bound_ticks, contract.upper_bound_ticks)
            else {
                return Err("futures band is missing".to_owned());
            };
            if contract.tick_value_micro <= 0 {
                return Err("tick_value_micro is not set".to_owned());
            }
            Payoff::Banded {
                lower,
                upper,
                tick_value_micro: contract.tick_value_micro,
            }
        }
        ListingKind::Binary => {
            let rule = contract
                .payoff_rule
                .as_ref()
                .ok_or_else(|| "PAYOFF_RULE_MISSING".to_owned())?;
            parse_payoff_rule(rule, contract.payout_ticks)?
        }
    };
    Ok(Instrument {
        ticker: contract.ticker.clone(),
        payoff,
        min_price_ticks: contract.min_price_ticks,
        max_price_ticks: contract.max_price_ticks,
    })
}

/// CM-06: may this event run under PORTFOLIO margin? Returns every violation found.
///
/// 1. Every future's `divider` equals the event's `outcome_divider`.
/// 2. Every binary range rule states the same `outcome_divider` (written at backfill).
/// 3. All contracts share `settlement_source`, `expected_resolution_at` and the
///    rounding clause (`settlement_rule.rounding_step` = the event's `outcome_step`).
/// 4. Every binary has a valid `payoff_rule`, and every instrument passes engine validation.
/// 5. `outcome_kind` matches every contract (CATEGORICAL events hold only categorical binaries).
pub fn validate_portfolio_event(
    event: &ListingEvent,
    contracts: &[ListingContract],
) -> Result<(), Vec<Violation>> {
    let mut out = Vec::new();
    if contracts.is_empty() {
        out.push(violation(
            "NO_CONTRACTS",
            None,
            "the event lists no contracts",
        ));
        return Err(out);
    }
    let numeric = event.outcome_kind == OutcomeKind::Numeric;
    if numeric && event.outcome_divider.is_none() {
        out.push(violation(
            "OUTCOME_DIVIDER_MISSING",
            None,
            "a numeric PORTFOLIO event needs outcome_divider",
        ));
    }
    if !numeric && event.grid != OutcomeGrid::INTEGERS {
        out.push(violation(
            "INVALID_GRID",
            None,
            "categorical events have no outcome grid",
        ));
    }

    let first = &contracts[0];
    let mut instruments = Vec::with_capacity(contracts.len());
    for contract in contracts {
        let ticker = Some(contract.ticker.as_str());
        // 3. one source, one resolution time, one rounding clause.
        if contract.settlement_source != first.settlement_source {
            out.push(violation(
                "SETTLEMENT_SOURCE_MISMATCH",
                ticker,
                format!("differs from {}", first.ticker),
            ));
        }
        if contract.expected_resolution_at != first.expected_resolution_at {
            out.push(violation(
                "RESOLUTION_TIME_MISMATCH",
                ticker,
                format!(
                    "{} vs {} on {}",
                    contract.expected_resolution_at, first.expected_resolution_at, first.ticker
                ),
            ));
        }
        if numeric {
            let rounding = contract
                .settlement_rule
                .as_ref()
                .and_then(|rule| rule.get("rounding_step"))
                .and_then(Value::as_i64);
            if rounding != Some(event.grid.step) {
                out.push(violation(
                    "ROUNDING_MISMATCH",
                    ticker,
                    format!(
                        "settlement_rule.rounding_step is {rounding:?}, the event's outcome_step is {}",
                        event.grid.step
                    ),
                ));
            }
        }
        // 1, 2 and 5: one axis, in the same ticks.
        match (contract.kind, numeric) {
            (ListingKind::Future, false) => out.push(violation(
                "OUTCOME_KIND_MISMATCH",
                ticker,
                "a future cannot sit on a categorical event",
            )),
            (ListingKind::Future, true) => {
                if event.outcome_divider.is_some_and(|d| d != contract.divider) {
                    out.push(violation(
                        "DIVIDER_MISMATCH",
                        ticker,
                        format!(
                            "future divider {} differs from outcome_divider {:?}",
                            contract.divider, event.outcome_divider
                        ),
                    ));
                }
            }
            (ListingKind::Binary, _) => {
                let rule_type = contract
                    .payoff_rule
                    .as_ref()
                    .and_then(|rule| rule.get("type"))
                    .and_then(Value::as_str);
                match (rule_type, numeric) {
                    (Some("range"), false) | (Some("categories"), true) => out.push(violation(
                        "OUTCOME_KIND_MISMATCH",
                        ticker,
                        format!(
                            "a {} rule on a {:?} event",
                            rule_type.unwrap_or("?"),
                            event.outcome_kind
                        ),
                    )),
                    (Some("range"), true) => {
                        let rule_divider = contract
                            .payoff_rule
                            .as_ref()
                            .and_then(|rule| rule.get("outcome_divider"))
                            .and_then(Value::as_i64);
                        if event.outcome_divider.is_some() && rule_divider != event.outcome_divider
                        {
                            out.push(violation(
                                "RULE_DIVIDER_MISMATCH",
                                ticker,
                                format!(
                                    "payoff_rule.outcome_divider is {rule_divider:?}, the event's is {:?}",
                                    event.outcome_divider
                                ),
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        // 4. machine-readable payoff that passes engine validation.
        match instrument(contract) {
            Ok(item) => instruments.push(item),
            Err(reason) => {
                let code = if reason == "PAYOFF_RULE_MISSING" {
                    "PAYOFF_RULE_MISSING"
                } else {
                    "INVALID_PAYOFF"
                };
                out.push(violation(code, ticker, reason));
            }
        }
    }
    if out.is_empty() {
        let grid = numeric.then_some(event.grid);
        if let Err(error) = super::EventBook::new(event.outcome_kind, instruments, grid) {
            out.push(violation("ENGINE_REJECTED", None, error.to_string()));
        }
    }
    if out.is_empty() {
        Ok(())
    } else {
        Err(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const USDC: i64 = 1_000_000;

    fn cpi_event() -> ListingEvent {
        ListingEvent {
            event_ticker: "XEV-CPI".into(),
            outcome_kind: OutcomeKind::Numeric,
            outcome_divider: Some(100),
            grid: OutcomeGrid {
                step: 10,
                anchor: 0,
                min: None,
                max: None,
            },
        }
    }

    fn future() -> ListingContract {
        ListingContract {
            ticker: "SXF-CPI".into(),
            kind: ListingKind::Future,
            divider: 100,
            lower_bound_ticks: Some(400),
            upper_bound_ticks: Some(600),
            tick_value_micro: 10 * USDC,
            min_price_ticks: 400,
            max_price_ticks: 600,
            payout_ticks: 100,
            payoff_rule: None,
            settlement_source: "BLS CPI-U NSA, first release".into(),
            expected_resolution_at: "2026-10-14T12:30:00Z".into(),
            settlement_rule: Some(json!({"type":"scalar_numeric","rounding_step":10})),
        }
    }

    fn binary() -> ListingContract {
        ListingContract {
            ticker: "SX-CPI-T450".into(),
            kind: ListingKind::Binary,
            divider: 100,
            lower_bound_ticks: Some(1),
            upper_bound_ticks: Some(99),
            tick_value_micro: 0,
            min_price_ticks: 1,
            max_price_ticks: 99,
            payout_ticks: 100,
            payoff_rule: Some(json!({"type":"range","lo":451,"hi":null,"outcome_divider":100})),
            settlement_rule: Some(json!({"type":"binary_event","rounding_step":10})),
            ..future()
        }
    }

    fn codes(result: Result<(), Vec<Violation>>) -> Vec<&'static str> {
        result
            .err()
            .unwrap_or_default()
            .iter()
            .map(|v| v.code)
            .collect()
    }

    #[test]
    fn a_consistent_event_passes() {
        assert_eq!(
            validate_portfolio_event(&cpi_event(), &[future(), binary()]),
            Ok(())
        );
    }

    #[test]
    fn a_categorical_fomc_event_passes() {
        // Stage 1 covers categorical events (decided 7 Oct): FOMC buckets, one
        // outcome, no outcome grid, no futures, every binary with yes_values.
        let fomc = ListingEvent {
            event_ticker: "XEV-FOMC".into(),
            outcome_kind: OutcomeKind::Categorical,
            outcome_divider: None,
            grid: OutcomeGrid::INTEGERS,
        };
        let bucket = |ticker: &str, value: &str| ListingContract {
            ticker: ticker.into(),
            payoff_rule: Some(json!({"type":"categories","yes_values":[value]})),
            settlement_rule: Some(json!({"type":"binary_event"})),
            ..binary()
        };
        let buckets = [
            bucket("FED-CUT", "CUT"),
            bucket("FED-HOLD", "HOLD"),
            bucket("FED-H25", "HIKE_25"),
            bucket("FED-HGT25", "HIKE_GT25"),
        ];
        assert_eq!(validate_portfolio_event(&fomc, &buckets), Ok(()));
        let with_grid = ListingEvent {
            grid: OutcomeGrid {
                step: 25,
                ..OutcomeGrid::INTEGERS
            },
            ..fomc.clone()
        };
        assert!(codes(validate_portfolio_event(&with_grid, &buckets)).contains(&"INVALID_GRID"));
    }

    #[test]
    fn each_violation_is_reported() {
        // 1. future divider differs from the event axis.
        let tenths = ListingContract {
            divider: 10,
            ..future()
        };
        assert!(
            codes(validate_portfolio_event(&cpi_event(), &[tenths, binary()]))
                .contains(&"DIVIDER_MISMATCH")
        );
        // 2. binary strike written on another scale.
        let strike = ListingContract {
            payoff_rule: Some(json!({"type":"range","lo":45,"hi":null,"outcome_divider":10})),
            ..binary()
        };
        assert!(
            codes(validate_portfolio_event(&cpi_event(), &[future(), strike]))
                .contains(&"RULE_DIVIDER_MISMATCH")
        );
        // 3. different source, resolution time or rounding.
        let source = ListingContract {
            settlement_source: "BLS".into(),
            ..binary()
        };
        let time = ListingContract {
            expected_resolution_at: "2026-11-01T23:59:00Z".into(),
            ..binary()
        };
        let rounding = ListingContract {
            settlement_rule: Some(json!({"type":"binary_event"})),
            ..binary()
        };
        assert!(
            codes(validate_portfolio_event(&cpi_event(), &[future(), source]))
                .contains(&"SETTLEMENT_SOURCE_MISMATCH")
        );
        assert!(
            codes(validate_portfolio_event(&cpi_event(), &[future(), time]))
                .contains(&"RESOLUTION_TIME_MISMATCH")
        );
        assert!(codes(validate_portfolio_event(
            &cpi_event(),
            &[future(), rounding]
        ))
        .contains(&"ROUNDING_MISMATCH"));
        // 4. a binary without a machine-readable payoff.
        let missing = ListingContract {
            payoff_rule: None,
            ..binary()
        };
        assert!(
            codes(validate_portfolio_event(&cpi_event(), &[future(), missing]))
                .contains(&"PAYOFF_RULE_MISSING")
        );
        // 5. outcome kind mismatch.
        let bucket = ListingContract {
            payoff_rule: Some(json!({"type":"categories","yes_values":["HIKE_25"]})),
            ..binary()
        };
        assert!(
            codes(validate_portfolio_event(&cpi_event(), &[future(), bucket]))
                .contains(&"OUTCOME_KIND_MISMATCH")
        );
        let categorical = ListingEvent {
            outcome_kind: OutcomeKind::Categorical,
            outcome_divider: None,
            grid: OutcomeGrid::INTEGERS,
            ..cpi_event()
        };
        assert!(codes(validate_portfolio_event(&categorical, &[future()]))
            .contains(&"OUTCOME_KIND_MISMATCH"));
        // A numeric event needs its divider.
        let no_divider = ListingEvent {
            outcome_divider: None,
            ..cpi_event()
        };
        assert!(codes(validate_portfolio_event(&no_divider, &[future()]))
            .contains(&"OUTCOME_DIVIDER_MISSING"));
    }
}
