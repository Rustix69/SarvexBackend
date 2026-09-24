use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicroUsdc(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceTicks(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractKind {
    Binary,
    Scalar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractState {
    Draft,
    Listed,
    Open,
    Halted,
    Closed,
    Resolving,
    Settled,
    Cancelled,
}

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid contract state: {0}")]
    InvalidContractState(String),
    #[error("invalid price ticks")]
    InvalidPrice,
    #[error("invalid quantity")]
    InvalidQuantity,
    #[error("invalid scalar contract band or tick value")]
    InvalidScalarContract,
    #[error("scalar calculation overflow")]
    Overflow,
}

/// Returns the stored cash value of one raw price tick.
///
/// `divider` is display metadata only. Listing code should calculate and
/// persist this value once, then all trading and settlement paths use it.
pub fn tick_value_micro(
    multiplier_micro_per_display_unit: i64,
    divider: i64,
) -> Result<i64, DomainError> {
    if multiplier_micro_per_display_unit <= 0 || divider <= 0 {
        return Err(DomainError::InvalidScalarContract);
    }
    if multiplier_micro_per_display_unit % divider != 0 {
        return Err(DomainError::InvalidScalarContract);
    }
    Ok(multiplier_micro_per_display_unit / divider)
}

/// Cash locked for one scalar opening direction at a fill/limit price.
pub fn scalar_lock_micro(
    is_long: bool,
    price_ticks: i64,
    quantity: i64,
    lower_ticks: i64,
    upper_ticks: i64,
    tick_value_micro: i64,
) -> Result<i64, DomainError> {
    if quantity <= 0 || tick_value_micro <= 0 || lower_ticks >= upper_ticks {
        return Err(DomainError::InvalidScalarContract);
    }
    let distance = if is_long {
        price_ticks.checked_sub(lower_ticks)
    } else {
        upper_ticks.checked_sub(price_ticks)
    }
    .filter(|value| *value >= 0)
    .ok_or(DomainError::InvalidPrice)?;
    let amount = i128::from(distance)
        .checked_mul(i128::from(quantity))
        .and_then(|value| value.checked_mul(i128::from(tick_value_micro)))
        .ok_or(DomainError::Overflow)?;
    i64::try_from(amount).map_err(|_| DomainError::Overflow)
}

/// Gross scalar settlement payout for a signed position. Positive quantity is
/// long, negative quantity is short. Resolution is clamped to the contract
/// band before multiplying.
pub fn scalar_payout_micro(
    signed_quantity: i64,
    settlement_ticks: i64,
    lower_ticks: i64,
    upper_ticks: i64,
    tick_value_micro: i64,
) -> Result<i64, DomainError> {
    if signed_quantity == 0 || tick_value_micro <= 0 || lower_ticks >= upper_ticks {
        return Err(DomainError::InvalidScalarContract);
    }
    let final_ticks = settlement_ticks.clamp(lower_ticks, upper_ticks);
    let distance = if signed_quantity > 0 {
        final_ticks - lower_ticks
    } else {
        upper_ticks - final_ticks
    };
    let amount = i128::from(signed_quantity.unsigned_abs())
        .checked_mul(i128::from(distance))
        .and_then(|value| value.checked_mul(i128::from(tick_value_micro)))
        .ok_or(DomainError::Overflow)?;
    i64::try_from(amount).map_err(|_| DomainError::Overflow)
}

/// Display-only mark-to-entry PnL for a signed scalar position.
pub fn scalar_pnl_micro(
    signed_quantity: i64,
    entry_ticks: i64,
    mark_ticks: i64,
    lower_ticks: i64,
    upper_ticks: i64,
    tick_value_micro: i64,
) -> Result<i64, DomainError> {
    if signed_quantity == 0 || tick_value_micro <= 0 || lower_ticks >= upper_ticks {
        return Err(DomainError::InvalidScalarContract);
    }
    let mark = mark_ticks.clamp(lower_ticks, upper_ticks);
    let delta = if signed_quantity > 0 {
        mark.checked_sub(entry_ticks)
    } else {
        entry_ticks.checked_sub(mark)
    }
    .ok_or(DomainError::Overflow)?;
    let amount = i128::from(signed_quantity.unsigned_abs())
        .checked_mul(i128::from(delta))
        .and_then(|value| value.checked_mul(i128::from(tick_value_micro)))
        .ok_or(DomainError::Overflow)?;
    i64::try_from(amount).map_err(|_| DomainError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_ttf_tick_value_without_extra_percent_scaling() {
        assert_eq!(tick_value_micro(10_000_000, 100).unwrap(), 100_000);
        assert_eq!(
            scalar_lock_micro(true, 4_000, 1, 2_000, 8_000, 100_000).unwrap(),
            200_000_000
        );
        assert_eq!(
            scalar_payout_micro(1, 6_000, 2_000, 8_000, 100_000).unwrap(),
            400_000_000
        );
    }

    #[test]
    fn clamps_settlement_and_mark_to_band() {
        assert_eq!(
            scalar_payout_micro(-2, 10_000, 2_000, 8_000, 100_000).unwrap(),
            0
        );
        assert_eq!(
            scalar_pnl_micro(1, 4_000, 10_000, 2_000, 8_000, 100_000).unwrap(),
            400_000_000
        );
    }
}
