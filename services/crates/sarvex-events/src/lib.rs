use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const EXECUTION_STREAM: &str = "sarvex:stream:execution";
pub const LEDGER_STREAM: &str = "sarvex:stream:ledger";
pub const POSITION_STREAM: &str = "sarvex:stream:position";
pub const MARKETDATA_STREAM: &str = "sarvex:stream:marketdata";
pub const SETTLEMENT_STREAM: &str = "sarvex:stream:settlement";
pub const AUDIT_STREAM: &str = "sarvex:stream:audit";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    pub event_id: String,
    pub event_type: String,
    pub global_seq: u64,
    pub contract_seq: Option<u64>,
    pub occurred_at: DateTime<Utc>,
    pub payload: T,
}
