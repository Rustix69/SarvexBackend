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
}
