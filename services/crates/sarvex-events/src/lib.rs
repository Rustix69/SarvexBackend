use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{env, time::Duration};

pub const EXEC_EVENTS: &str = "exec.events";
pub const LEDGER_EVENTS: &str = "ledger.events";
pub const MARKETDATA_EVENTS: &str = "md.events";
pub const SETTLEMENT_EVENTS: &str = "settlement.events";
pub const AUDIT_EVENTS: &str = "audit.events";

const RETAINED_STREAM: &str = "SARVEX_EVENTS";

#[derive(Clone)]
pub struct EventPublisher {
    client: async_nats::Client,
    retained: Option<async_nats::jetstream::Context>,
}

impl EventPublisher {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = async_nats::connect(url).await?;
        let retained = if env::var("EVENT_RETENTION")
            .unwrap_or_else(|_| "core".to_owned())
            .eq_ignore_ascii_case("jetstream")
        {
            let context = async_nats::jetstream::new(client.clone());
            context
                .get_or_create_stream(async_nats::jetstream::stream::Config {
                    name: RETAINED_STREAM.to_owned(),
                    subjects: vec![
                        "md.>".to_owned(),
                        "exec.>".to_owned(),
                        "oracle.>".to_owned(),
                        "settlement.>".to_owned(),
                        "ledger.>".to_owned(),
                    ],
                    retention: async_nats::jetstream::stream::RetentionPolicy::Limits,
                    storage: async_nats::jetstream::stream::StorageType::File,
                    max_age: Duration::from_secs(
                        env::var("EVENT_RETENTION_SECONDS")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(30 * 24 * 60 * 60),
                    ),
                    max_bytes: env::var("EVENT_RETENTION_MAX_BYTES")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(10 * 1024 * 1024 * 1024),
                    max_messages: env::var("EVENT_RETENTION_MAX_MESSAGES")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(50_000_000),
                    ..Default::default()
                })
                .await?;
            Some(context)
        } else {
            None
        };
        Ok(Self { client, retained })
    }

    pub async fn publish(&self, subject: String, payload: Vec<u8>) -> anyhow::Result<()> {
        if let Some(context) = &self.retained {
            context.publish(subject, payload.into()).await?.await?;
        } else {
            self.client.publish(subject, payload.into()).await?;
            self.client.flush().await?;
        }
        Ok(())
    }
}

pub fn execution_fills_subject(ticker: &str) -> String {
    format!("exec.fills.{ticker}")
}

pub fn market_book_subject(ticker: &str) -> String {
    format!("md.book.{ticker}")
}

pub fn market_trade_subject(ticker: &str) -> String {
    format!("md.trade.{ticker}")
}

pub fn market_ticker_subject(ticker: &str) -> String {
    format!("md.ticker.{ticker}")
}

pub fn user_execution_subject(user_id: &str) -> String {
    format!("exec.user.{user_id}")
}

pub fn user_fill_subject(user_id: &str) -> String {
    format!("exec.fills.user.{user_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    pub schema_version: u16,
    pub event_id: String,
    pub event_type: String,
    pub subject: String,
    pub global_seq: u64,
    pub contract_seq: Option<u64>,
    pub occurred_at: DateTime<Utc>,
    pub payload: T,
}

pub fn encode<T: Serialize>(event: &EventEnvelope<T>) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(event)
}

pub fn decode<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
) -> Result<EventEnvelope<T>, serde_json::Error> {
    serde_json::from_slice(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_are_stable_and_namespaced() {
        assert_eq!(execution_fills_subject("RBI-JUN26"), "exec.fills.RBI-JUN26");
        assert_eq!(market_book_subject("RBI-JUN26"), "md.book.RBI-JUN26");
        assert_eq!(user_fill_subject("u_1"), "exec.fills.user.u_1");
    }

    #[test]
    fn envelope_round_trips_with_sequence_metadata() {
        let event = EventEnvelope {
            schema_version: 1,
            event_id: "evt_1".to_owned(),
            event_type: "OrderAccepted".to_owned(),
            subject: EXEC_EVENTS.to_owned(),
            global_seq: 8,
            contract_seq: Some(3),
            occurred_at: Utc::now(),
            payload: serde_json::json!({ "order_id": "ord_1" }),
        };
        let decoded: EventEnvelope<serde_json::Value> =
            decode(&encode(&event).expect("encode")).expect("decode");
        assert_eq!(decoded.event_id, "evt_1");
        assert_eq!(decoded.global_seq, 8);
        assert_eq!(decoded.contract_seq, Some(3));
    }
}
