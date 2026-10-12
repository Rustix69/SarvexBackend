//! Stage 1 cross margin (model C3): fully funded portfolio margin per event.
//!
//! Every contract on an event is a function of one outcome `x` (the event's
//! resolved value in its own ticks, or a category). A contract is a *unit*
//! whose value at settlement is `V(x)`, between 0 and a known maximum, in
//! micro-USDC. Positions are signed quantities of units: a NO position is a
//! negative YES quantity and a short future is a negative long quantity.
//!
//! Cash model ("premium style"): on every fill the buyer pays `q * c(p)` to the
//! seller, where `c(p)` is the unit value at the trade price. At settlement an
//! account receives `sum(q_i * V_i(x))` when positive and pays it when
//! negative. Both legs are zero-sum across an event.
//!
//! The margin an account must lock for one event covers its positions and
//! every working order, for any combination of (partial) fills:
//!
//! ```text
//! M = max( sum_o max(0, k_o),
//!          max_x [ -sum_i q_i V_i(x) + sum_o max(0, k_o - q_o V_o(x)) ] )
//! ```
//!
//! with `k_o = q_o * c(p_o)` the premium order `o` pays at its limit price
//! (`q_o` > 0 buy, < 0 sell). Every `V_i` is piecewise linear in `x`, so the
//! maximum over `x` is reached at a kink; only those points are evaluated.
//!
//! Guarantees (all covered by tests in this module):
//! * `M` >= the true worst-case settlement obligation for every subset and
//!   fraction of working-order fills;
//! * `M` <= the separate (Model A style) margin of the same positions and orders;
//! * on an event with an outcome grid (CM-11), `M` is exact over the grid and
//!   settlement refuses off-grid outcomes;
//! * a fill at a price at least as good as its limit never needs new cash:
//!   `M_after + premium_paid <= M_before`;
//! * settlement amounts across all accounts of an event sum to zero.
//!
//! All arithmetic is integer: prices in raw ticks, money in micro-USDC,
//! intermediate products in `i128`. Nothing here performs I/O.

use std::collections::BTreeSet;
use thiserror::Error;

pub mod listing;

/// Value of one binary price tick (1 cent of a 1 USDC payout).
pub const BINARY_TICK_VALUE_MICRO: i64 = 10_000;
/// Payout of a winning binary contract, in ticks (100 ticks = 1 USDC).
pub const DEFAULT_BINARY_PAYOUT_TICKS: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    /// The event resolves to an integer (CPI in bp, NFP in thousands, a price).
    Numeric,
    /// The event resolves to one named category (e.g. FOMC "HIKE_25").
    Categorical,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    Numeric(i64),
    Category(String),
    /// Any category not named by a contract on the event.
    OtherCategory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payoff {
    /// Binary YES that pays `payout_ticks * tick_value_micro` when
    /// `lo <= x <= hi` (an open end is `None`). "CPI > 3.40%" is
    /// `lo = Some(341), hi = None`.
    BinaryRange {
        lo: Option<i64>,
        hi: Option<i64>,
        payout_ticks: i64,
        tick_value_micro: i64,
    },
    /// Binary YES that pays when the resolved category is one of `yes_values`.
    BinaryCategory {
        yes_values: Vec<String>,
        payout_ticks: i64,
        tick_value_micro: i64,
    },
    /// Banded future, long unit: `(clamp(x, lower, upper) - lower) * tick_value_micro`.
    Banded {
        lower: i64,
        upper: i64,
        tick_value_micro: i64,
    },
}

/// The outcomes a numeric event can contractually settle on (CM-11):
/// `{ anchor + n * step }`, intersected with `[min, max]` when those are set.
///
/// The settlement rule must state the rounding ("the official value rounded to
/// `step`, half away from zero"); otherwise the grid is not a contract term and
/// margin computed on it is not guaranteed to cover settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutcomeGrid {
    pub step: i64,
    pub anchor: i64,
    /// Set only when the outcome is naturally bounded or the rule clamps it.
    pub min: Option<i64>,
    pub max: Option<i64>,
}

impl OutcomeGrid {
    /// Every integer is a possible outcome (the behaviour without a grid).
    pub const INTEGERS: OutcomeGrid = OutcomeGrid {
        step: 1,
        anchor: 0,
        min: None,
        max: None,
    };

    pub fn contains(&self, x: i64) -> bool {
        (x - self.anchor).rem_euclid(self.step) == 0
            && self.min.is_none_or(|min| x >= min)
            && self.max.is_none_or(|max| x <= max)
    }

    fn validate(&self) -> Result<()> {
        let invalid = |reason: &str| MarginError::InvalidGrid(reason.to_owned());
        if self.step <= 0 {
            return Err(invalid("step must be positive"));
        }
        if let (Some(min), Some(max)) = (self.min, self.max) {
            if min > max {
                return Err(invalid("min is above max"));
            }
        }
        for bound in [self.min, self.max].into_iter().flatten() {
            if (bound - self.anchor).rem_euclid(self.step) != 0 {
                return Err(invalid("min and max must lie on the grid"));
            }
        }
        Ok(())
    }

    /// Replaces each raw kink by the grid points just below and just above it.
    ///
    /// Exact: between consecutive raw kinks the P&L is linear (or constant), so
    /// over the grid points of that piece its extremes sit at the grid points
    /// nearest the piece's ends; beyond the outermost kinks it is flat.
    fn snap(&self, raw: &BTreeSet<i64>) -> BTreeSet<i64> {
        let mut out = BTreeSet::new();
        for &k in raw {
            let below = self.anchor + (k - self.anchor).div_euclid(self.step) * self.step;
            out.insert(below);
            if below != k {
                out.insert(below + self.step);
            }
        }
        if let Some(min) = self.min {
            out.retain(|x| *x >= min);
            out.insert(min);
        }
        if let Some(max) = self.max {
            out.retain(|x| *x <= max);
            out.insert(max);
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instrument {
    pub ticker: String,
    pub payoff: Payoff,
    pub min_price_ticks: i64,
    pub max_price_ticks: i64,
}

impl Instrument {
    /// A standard binary: 1..=99 cents, pays 1 USDC.
    pub fn binary_range(ticker: impl Into<String>, lo: Option<i64>, hi: Option<i64>) -> Self {
        Self {
            ticker: ticker.into(),
            payoff: Payoff::BinaryRange {
                lo,
                hi,
                payout_ticks: DEFAULT_BINARY_PAYOUT_TICKS,
                tick_value_micro: BINARY_TICK_VALUE_MICRO,
            },
            min_price_ticks: 1,
            max_price_ticks: DEFAULT_BINARY_PAYOUT_TICKS - 1,
        }
    }

    /// A standard categorical binary: 1..=99 cents, pays 1 USDC.
    pub fn binary_category(ticker: impl Into<String>, yes_values: &[&str]) -> Self {
        Self {
            ticker: ticker.into(),
            payoff: Payoff::BinaryCategory {
                yes_values: yes_values.iter().map(|v| (*v).to_owned()).collect(),
                payout_ticks: DEFAULT_BINARY_PAYOUT_TICKS,
                tick_value_micro: BINARY_TICK_VALUE_MICRO,
            },
            min_price_ticks: 1,
            max_price_ticks: DEFAULT_BINARY_PAYOUT_TICKS - 1,
        }
    }

    /// A banded future tradable anywhere inside its band.
    pub fn banded(
        ticker: impl Into<String>,
        lower: i64,
        upper: i64,
        tick_value_micro: i64,
    ) -> Self {
        Self {
            ticker: ticker.into(),
            payoff: Payoff::Banded {
                lower,
                upper,
                tick_value_micro,
            },
            min_price_ticks: lower,
            max_price_ticks: upper,
        }
    }
}

/// Net signed quantity of one instrument held by an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub instrument: usize,
    pub qty: i64,
}

/// Remaining quantity of a working order: `qty` > 0 buys units, < 0 sells units.
/// Binary NO orders and futures SHORT orders must already be normalised to
/// YES/LONG units at the complementary price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkingOrder {
    pub instrument: usize,
    pub qty: i64,
    pub price_ticks: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventMargin {
    /// `M`: what the account must lock on this event for positions and orders.
    pub margin_micro: i64,
    /// `R`: what the positions alone could owe at settlement.
    pub position_requirement_micro: i64,
    /// What separate per-position / per-order (Model A) margin would lock.
    pub standalone_micro: i64,
    /// The outcome at which the positions owe the most (`None` when they can never owe).
    pub worst_outcome: Option<Outcome>,
}

impl EventMargin {
    pub fn order_margin_micro(&self) -> i64 {
        self.margin_micro - self.position_requirement_micro
    }

    pub fn savings_micro(&self) -> i64 {
        self.standalone_micro - self.margin_micro
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MarginError {
    #[error("instrument {ticker}: {reason}")]
    InvalidInstrument { ticker: String, reason: String },
    #[error("instrument {ticker} does not fit a {kind:?} event")]
    OutcomeKindMismatch { ticker: String, kind: OutcomeKind },
    #[error("unknown instrument index {0}")]
    UnknownInstrument(usize),
    #[error("price {price_ticks} is outside [{min}, {max}] for {ticker}")]
    InvalidPrice {
        ticker: String,
        price_ticks: i64,
        min: i64,
        max: i64,
    },
    #[error("order quantity must be non-zero")]
    ZeroQuantity,
    #[error("outcome {0:?} does not match the event's outcome kind")]
    InvalidOutcome(Outcome),
    #[error("invalid outcome grid: {0}")]
    InvalidGrid(String),
    #[error("settlement amounts do not sum to zero (off by {0} micro-USDC)")]
    NotZeroSum(i128),
    #[error("margin arithmetic overflow")]
    Overflow,
}

type Result<T> = std::result::Result<T, MarginError>;

impl Payoff {
    fn unit_value(&self) -> (i64, i64) {
        match self {
            Payoff::BinaryRange {
                payout_ticks,
                tick_value_micro,
                ..
            }
            | Payoff::BinaryCategory {
                payout_ticks,
                tick_value_micro,
                ..
            } => (*payout_ticks, *tick_value_micro),
            Payoff::Banded {
                lower,
                upper,
                tick_value_micro,
            } => (upper - lower, *tick_value_micro),
        }
    }

    /// Largest settlement value of one long unit.
    pub fn max_value_micro(&self) -> i128 {
        let (ticks, tick_value) = self.unit_value();
        i128::from(ticks) * i128::from(tick_value)
    }

    /// Settlement value of one long unit at outcome `x`.
    pub fn value_at(&self, x: &Outcome) -> Result<i128> {
        match (self, x) {
            (Payoff::BinaryRange { lo, hi, .. }, Outcome::Numeric(value)) => {
                let yes = lo.is_none_or(|lo| *value >= lo) && hi.is_none_or(|hi| *value <= hi);
                Ok(if yes { self.max_value_micro() } else { 0 })
            }
            (Payoff::BinaryCategory { yes_values, .. }, Outcome::Category(name)) => {
                Ok(if yes_values.iter().any(|v| v == name) {
                    self.max_value_micro()
                } else {
                    0
                })
            }
            (Payoff::BinaryCategory { .. }, Outcome::OtherCategory) => Ok(0),
            (
                Payoff::Banded {
                    lower,
                    upper,
                    tick_value_micro,
                },
                Outcome::Numeric(value),
            ) => {
                let clamped = (*value).clamp(*lower, *upper);
                Ok(i128::from(clamped - lower) * i128::from(*tick_value_micro))
            }
            (_, other) => Err(MarginError::InvalidOutcome(other.clone())),
        }
    }

    /// Value of one unit traded at `price_ticks` (the premium per unit).
    pub fn value_of_price(&self, price_ticks: i64) -> i128 {
        match self {
            Payoff::BinaryRange {
                tick_value_micro, ..
            }
            | Payoff::BinaryCategory {
                tick_value_micro, ..
            } => i128::from(price_ticks) * i128::from(*tick_value_micro),
            Payoff::Banded {
                lower,
                tick_value_micro,
                ..
            } => i128::from(price_ticks - lower) * i128::from(*tick_value_micro),
        }
    }
}

/// All instruments that share one event and its outcome.
#[derive(Debug, Clone)]
pub struct EventBook {
    kind: OutcomeKind,
    instruments: Vec<Instrument>,
    grid: Option<OutcomeGrid>,
}

impl EventBook {
    /// `grid` = the event's contractual outcome grid (numeric events only);
    /// `None` treats every integer as a possible outcome.
    pub fn new(
        kind: OutcomeKind,
        instruments: Vec<Instrument>,
        grid: Option<OutcomeGrid>,
    ) -> Result<Self> {
        for instrument in &instruments {
            validate_instrument(kind, instrument)?;
        }
        if let Some(grid) = &grid {
            if kind != OutcomeKind::Numeric {
                return Err(MarginError::InvalidGrid(
                    "only numeric events have an outcome grid".to_owned(),
                ));
            }
            grid.validate()?;
        }
        Ok(Self {
            kind,
            instruments,
            grid,
        })
    }

    pub fn grid(&self) -> Option<&OutcomeGrid> {
        self.grid.as_ref()
    }

    pub fn kind(&self) -> OutcomeKind {
        self.kind
    }

    pub fn instruments(&self) -> &[Instrument] {
        &self.instruments
    }

    pub fn instrument_index(&self, ticker: &str) -> Option<usize> {
        self.instruments.iter().position(|i| i.ticker == ticker)
    }

    fn instrument(&self, index: usize) -> Result<&Instrument> {
        self.instruments
            .get(index)
            .ok_or(MarginError::UnknownInstrument(index))
    }

    /// Signed premium an account pays for trading `qty` units at `price_ticks`
    /// (negative when it receives premium, i.e. sells).
    pub fn trade_value_micro(&self, instrument: usize, price_ticks: i64, qty: i64) -> Result<i64> {
        let item = self.instrument(instrument)?;
        check_price(item, price_ticks)?;
        narrow(
            item.payoff
                .value_of_price(price_ticks)
                .checked_mul(i128::from(qty))
                .ok_or(MarginError::Overflow)?,
        )
    }

    /// `R`: the most the positions could owe at settlement.
    pub fn requirement(&self, positions: &[Position]) -> Result<i64> {
        Ok(self
            .event_margin(positions, &[])?
            .position_requirement_micro)
    }

    /// `M`, `R`, the standalone comparison and the worst outcome.
    pub fn event_margin(
        &self,
        positions: &[Position],
        orders: &[WorkingOrder],
    ) -> Result<EventMargin> {
        let net = self.net_positions(positions)?;
        let mut priced: Vec<(usize, i128, i128)> = Vec::with_capacity(orders.len());
        for order in orders {
            let item = self.instrument(order.instrument)?;
            if order.qty == 0 {
                return Err(MarginError::ZeroQuantity);
            }
            check_price(item, order.price_ticks)?;
            let premium = item
                .payoff
                .value_of_price(order.price_ticks)
                .checked_mul(i128::from(order.qty))
                .ok_or(MarginError::Overflow)?;
            priced.push((order.instrument, i128::from(order.qty), premium));
        }

        let premium_branch = priced
            .iter()
            .try_fold(0_i128, |sum, (_, _, k)| sum.checked_add((*k).max(0)))
            .ok_or(MarginError::Overflow)?;
        let mut margin = premium_branch;
        let mut requirement = 0_i128;
        let mut worst = None;

        for x in self.candidates(orders) {
            let mut owed = 0_i128;
            for (index, qty) in net.iter().enumerate() {
                if *qty != 0 {
                    let value = self.instruments[index].payoff.value_at(&x)?;
                    owed = owed
                        .checked_sub(
                            value
                                .checked_mul(i128::from(*qty))
                                .ok_or(MarginError::Overflow)?,
                        )
                        .ok_or(MarginError::Overflow)?;
                }
            }
            if owed > requirement {
                requirement = owed;
                worst = Some(x.clone());
            }
            let mut stressed = owed;
            for (index, qty, premium) in &priced {
                let value = self.instruments[*index].payoff.value_at(&x)?;
                let hurt = premium
                    .checked_sub(value.checked_mul(*qty).ok_or(MarginError::Overflow)?)
                    .ok_or(MarginError::Overflow)?;
                stressed = stressed
                    .checked_add(hurt.max(0))
                    .ok_or(MarginError::Overflow)?;
            }
            margin = margin.max(stressed);
        }

        let standalone = self.standalone(&net, &priced)?;
        Ok(EventMargin {
            margin_micro: narrow(margin)?,
            position_requirement_micro: narrow(requirement)?,
            standalone_micro: narrow(standalone)?,
            worst_outcome: worst,
        })
    }

    /// Largest amount the positions alone can cost the account at settlement, found by
    /// settling them at every candidate outcome. Same quantity as
    /// `EventMargin::position_requirement_micro`, but computed through `settlement_micro`,
    /// so the shadow log can check one code path against the other.
    pub fn worst_position_loss_micro(&self, positions: &[Position]) -> Result<i64> {
        let mut worst = 0_i64;
        for x in self.candidates(&[]) {
            worst = worst.max(self.settlement_micro(positions, &x)?.saturating_neg());
        }
        Ok(worst)
    }

    /// Net amount one account receives (positive) or pays (negative) at settlement.
    pub fn settlement_micro(&self, positions: &[Position], outcome: &Outcome) -> Result<i64> {
        self.check_outcome(outcome)?;
        let net = self.net_positions(positions)?;
        let mut total = 0_i128;
        for (index, qty) in net.iter().enumerate() {
            if *qty != 0 {
                let value = self.instruments[index].payoff.value_at(outcome)?;
                total = total
                    .checked_add(
                        value
                            .checked_mul(i128::from(*qty))
                            .ok_or(MarginError::Overflow)?,
                    )
                    .ok_or(MarginError::Overflow)?;
            }
        }
        narrow(total)
    }

    /// Settlement for every account on the event; refuses unless the amounts net to zero.
    pub fn settle_accounts(
        &self,
        accounts: &[Vec<Position>],
        outcome: &Outcome,
    ) -> Result<Vec<i64>> {
        let amounts = accounts
            .iter()
            .map(|positions| self.settlement_micro(positions, outcome))
            .collect::<Result<Vec<_>>>()?;
        let total: i128 = amounts.iter().map(|v| i128::from(*v)).sum();
        if total != 0 {
            return Err(MarginError::NotZeroSum(total));
        }
        Ok(amounts)
    }

    /// Rejects an outcome of the wrong kind, or off the event's outcome grid.
    pub fn check_outcome(&self, outcome: &Outcome) -> Result<()> {
        match (self.kind, outcome) {
            (OutcomeKind::Numeric, Outcome::Numeric(x)) => {
                if self.grid.is_some_and(|grid| !grid.contains(*x)) {
                    Err(MarginError::InvalidOutcome(outcome.clone()))
                } else {
                    Ok(())
                }
            }
            (OutcomeKind::Categorical, Outcome::Category(_) | Outcome::OtherCategory) => Ok(()),
            _ => Err(MarginError::InvalidOutcome(outcome.clone())),
        }
    }

    fn net_positions(&self, positions: &[Position]) -> Result<Vec<i64>> {
        let mut net = vec![0_i64; self.instruments.len()];
        for position in positions {
            let slot = net
                .get_mut(position.instrument)
                .ok_or(MarginError::UnknownInstrument(position.instrument))?;
            *slot = slot
                .checked_add(position.qty)
                .ok_or(MarginError::Overflow)?;
        }
        Ok(net)
    }

    /// Outcomes where the margin function can reach its maximum: every kink of
    /// every payoff, plus futures order prices (where `max(0, k - qV)` bends).
    fn candidates(&self, orders: &[WorkingOrder]) -> Vec<Outcome> {
        match self.kind {
            OutcomeKind::Categorical => {
                let mut names = BTreeSet::new();
                for instrument in &self.instruments {
                    if let Payoff::BinaryCategory { yes_values, .. } = &instrument.payoff {
                        names.extend(yes_values.iter().cloned());
                    }
                }
                let mut out: Vec<Outcome> = names.into_iter().map(Outcome::Category).collect();
                out.push(Outcome::OtherCategory);
                out
            }
            OutcomeKind::Numeric => {
                let mut points = BTreeSet::new();
                for instrument in &self.instruments {
                    match &instrument.payoff {
                        Payoff::BinaryRange { lo, hi, .. } => {
                            if let Some(lo) = lo {
                                points.insert(*lo);
                                if let Some(below) = lo.checked_sub(1) {
                                    points.insert(below);
                                }
                            }
                            if let Some(hi) = hi {
                                points.insert(*hi);
                                if let Some(above) = hi.checked_add(1) {
                                    points.insert(above);
                                }
                            }
                        }
                        Payoff::Banded { lower, upper, .. } => {
                            points.insert(*lower);
                            points.insert(*upper);
                        }
                        Payoff::BinaryCategory { .. } => {}
                    }
                }
                for order in orders {
                    if let Some(Payoff::Banded { .. }) =
                        self.instruments.get(order.instrument).map(|i| &i.payoff)
                    {
                        points.insert(order.price_ticks);
                    }
                }
                if points.is_empty() {
                    points.insert(self.grid.map_or(0, |grid| grid.anchor));
                }
                if let Some(grid) = &self.grid {
                    points = grid.snap(&points);
                }
                points.into_iter().map(Outcome::Numeric).collect()
            }
        }
    }

    /// What today's separate (Model A) margin locks, on the same basis as `M`
    /// (premium excluded): short positions lock their full max value; an
    /// opening buy order locks its premium; an opening sell order locks max
    /// value minus the premium it would receive.
    ///
    /// Closing orders (CM-09) mirror `risk-svc::required_hold` today:
    /// * futures: the part of an order that reduces the position locks
    ///   nothing. Per instrument, closing capacity starts at `|net position|`;
    ///   orders on the opposite side consume it in the order given (callers
    ///   pass orders in time priority); only the opening remainder is margined;
    /// * binaries: the hold depends on the action alone, so a closing order
    ///   still locks its full opening amount.
    fn standalone(&self, net: &[i64], priced: &[(usize, i128, i128)]) -> Result<i128> {
        let mut total = 0_i128;
        for (index, qty) in net.iter().enumerate() {
            if *qty < 0 {
                let lock = self.instruments[index]
                    .payoff
                    .max_value_micro()
                    .checked_mul(i128::from(qty.unsigned_abs()))
                    .ok_or(MarginError::Overflow)?;
                total = total.checked_add(lock).ok_or(MarginError::Overflow)?;
            }
        }
        let mut capacity: Vec<i128> = net.iter().map(|q| i128::from(q.unsigned_abs())).collect();
        for (index, qty, premium) in priced {
            let payoff = &self.instruments[*index].payoff;
            let size = qty.abs();
            let closing = match payoff {
                Payoff::Banded { .. } if net[*index] != 0 && (net[*index] > 0) != (*qty > 0) => {
                    let closing = size.min(capacity[*index]);
                    capacity[*index] -= closing;
                    closing
                }
                _ => 0,
            };
            let opening = size - closing;
            if opening == 0 {
                continue;
            }
            // `premium` = qty * unit value at the limit price, so this division is exact.
            let unit_premium = premium / qty;
            let unit_lock = if *qty > 0 {
                unit_premium
            } else {
                payoff
                    .max_value_micro()
                    .checked_sub(unit_premium)
                    .ok_or(MarginError::Overflow)?
            };
            let lock = unit_lock
                .checked_mul(opening)
                .ok_or(MarginError::Overflow)?;
            total = total.checked_add(lock).ok_or(MarginError::Overflow)?;
        }
        Ok(total)
    }
}

fn validate_instrument(kind: OutcomeKind, instrument: &Instrument) -> Result<()> {
    let invalid = |reason: &str| MarginError::InvalidInstrument {
        ticker: instrument.ticker.clone(),
        reason: reason.to_owned(),
    };
    if instrument.min_price_ticks > instrument.max_price_ticks {
        return Err(invalid("min price is above max price"));
    }
    match (&instrument.payoff, kind) {
        (
            Payoff::BinaryRange {
                lo,
                hi,
                payout_ticks,
                tick_value_micro,
            },
            OutcomeKind::Numeric,
        ) => {
            if *payout_ticks <= 0 || *tick_value_micro <= 0 {
                return Err(invalid("payout and tick value must be positive"));
            }
            if let (Some(lo), Some(hi)) = (lo, hi) {
                if lo > hi {
                    return Err(invalid("YES range is empty (lo > hi)"));
                }
            }
            if instrument.min_price_ticks < 0 || instrument.max_price_ticks > *payout_ticks {
                return Err(invalid("prices must lie within [0, payout]"));
            }
        }
        (
            Payoff::BinaryCategory {
                yes_values,
                payout_ticks,
                tick_value_micro,
            },
            OutcomeKind::Categorical,
        ) => {
            if *payout_ticks <= 0 || *tick_value_micro <= 0 {
                return Err(invalid("payout and tick value must be positive"));
            }
            if yes_values.is_empty() {
                return Err(invalid("categorical binary needs at least one YES value"));
            }
            if instrument.min_price_ticks < 0 || instrument.max_price_ticks > *payout_ticks {
                return Err(invalid("prices must lie within [0, payout]"));
            }
        }
        (
            Payoff::Banded {
                lower,
                upper,
                tick_value_micro,
            },
            OutcomeKind::Numeric,
        ) => {
            if lower >= upper || *tick_value_micro <= 0 {
                return Err(invalid("band must be non-empty and tick value positive"));
            }
            if instrument.min_price_ticks < *lower || instrument.max_price_ticks > *upper {
                return Err(invalid("prices must lie within the band"));
            }
        }
        (_, kind) => {
            return Err(MarginError::OutcomeKindMismatch {
                ticker: instrument.ticker.clone(),
                kind,
            })
        }
    }
    Ok(())
}

fn check_price(instrument: &Instrument, price_ticks: i64) -> Result<()> {
    if price_ticks < instrument.min_price_ticks || price_ticks > instrument.max_price_ticks {
        return Err(MarginError::InvalidPrice {
            ticker: instrument.ticker.clone(),
            price_ticks,
            min: instrument.min_price_ticks,
            max: instrument.max_price_ticks,
        });
    }
    Ok(())
}

fn narrow(value: i128) -> Result<i64> {
    i64::try_from(value).map_err(|_| MarginError::Overflow)
}

#[cfg(test)]
mod tests;
