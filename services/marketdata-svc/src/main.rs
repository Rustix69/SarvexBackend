use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use sarvex_contracts::sarvex::v1::{
    execution_event::Event, matching_engine_client::MatchingEngineClient, StreamExecutionsRequest,
};
use sarvex_events::{market_book_subject, market_trade_subject, EventEnvelope, EventPublisher};
use std::{env, time::Duration};
use tonic::transport::{Channel, Endpoint};

#[tokio::main]
async fn main() -> Result<()> {
    sarvex_runtime::init_tracing("marketdata-svc");
    let me_addr = env::var("ME_CORE_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50054".to_owned());
    let nats_url = env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned());
    tokio::spawn(run_publisher(me_addr, nats_url));
    sarvex_runtime::run_health_service("marketdata-svc").await
}

async fn connect_me(address: &str) -> Result<MatchingEngineClient<Channel>> {
    let endpoint = Endpoint::from_shared(address.to_owned())?;
    Ok(MatchingEngineClient::connect(endpoint).await?)
}

async fn run_publisher(me_addr: String, nats_url: String) {
    let mut last_seq = 0;
    loop {
        let publisher = match EventPublisher::connect(&nats_url).await {
            Ok(publisher) => publisher,
            Err(error) => {
                tracing::warn!(error = %error, "market-data NATS connection failed");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let mut me = match connect_me(&me_addr).await {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(error = %error, "market-data me-core connection failed");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let response = match me
            .stream_executions(StreamExecutionsRequest {
                from_global_seq: last_seq,
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(error = %error, "market-data execution stream failed");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let mut stream = response.into_inner();
        while let Some(next) = stream.next().await {
            let event = match next {
                Ok(event) => event,
                Err(error) => {
                    tracing::warn!(error = %error, "market-data execution stream disconnected");
                    break;
                }
            };
            let event_seq = event.global_seq;
            if let Err(error) = publish_event(&publisher, event).await {
                tracing::warn!(error = %error, "market-data event publish failed");
                break;
            }
            // Advance only after the NATS flush succeeds. A reconnect then
            // replays the failed event instead of creating a sequence gap.
            last_seq = last_seq.max(event_seq);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn publish_event(
    publisher: &EventPublisher,
    event: sarvex_contracts::sarvex::v1::ExecutionEvent,
) -> Result<()> {
    let ticker = event.ticker.clone();
    let occurred_at = event
        .ts
        .as_ref()
        .and_then(|timestamp| {
            DateTime::<Utc>::from_timestamp(timestamp.seconds, timestamp.nanos as u32)
        })
        .unwrap_or_else(Utc::now);
    match event.event {
        Some(Event::Fill(fill)) => {
            let payload = serde_json::json!({
                "ticker": ticker,
                "fill_id": fill.fill_id,
                "price_ticks": fill.price_ticks,
                "count": fill.count,
                "aggressor_side": fill.aggressor_side,
                "global_seq": event.global_seq,
                "contract_seq": event.contract_seq,
                "occurred_at": occurred_at,
            });
            let envelope = EventEnvelope {
                schema_version: 1,
                event_id: fill.fill_id,
                event_type: "MarketTrade".to_owned(),
                subject: market_trade_subject(&event.ticker),
                global_seq: event.global_seq,
                contract_seq: Some(event.contract_seq),
                occurred_at,
                payload,
            };
            let bytes = serde_json::to_vec(&envelope)?;
            publisher
                .publish(market_trade_subject(&event.ticker), bytes)
                .await?;
        }
        Some(Event::BookDelta(delta)) => {
            let payload = serde_json::json!({
                "ticker": ticker,
                "side": delta.side,
                "price_ticks": delta.price_ticks,
                "qty_delta": delta.qty_delta,
                "new_total_qty": delta.new_total_qty,
                "global_seq": event.global_seq,
                "contract_seq": event.contract_seq,
                "occurred_at": occurred_at,
            });
            let envelope = EventEnvelope {
                schema_version: 1,
                event_id: format!("book_{}", event.global_seq),
                event_type: "BookDelta".to_owned(),
                subject: market_book_subject(&event.ticker),
                global_seq: event.global_seq,
                contract_seq: Some(event.contract_seq),
                occurred_at,
                payload,
            };
            let bytes = serde_json::to_vec(&envelope)?;
            publisher
                .publish(market_book_subject(&event.ticker), bytes)
                .await?;
        }
        _ => {}
    }
    Ok(())
}
