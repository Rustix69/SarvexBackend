use anyhow::{Context, Result};
use sarvex_contracts::sarvex::v1::{
    Action, AddBookRequest, ContractKind, MeSubmitOrderRequest, Side,
};
use sarvex_me_client::MeCoreClient;
use std::{
    env,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

#[tokio::main]
async fn main() -> Result<()> {
    let address = env::var("ME_CORE_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50054".into());
    let orders: usize = env::var("LOADTEST_ORDERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let concurrency: usize = env::var("LOADTEST_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64);
    let ticker = env::var("LOADTEST_TICKER").unwrap_or_else(|_| "LOADTEST-BINARY".into());
    let client = MeCoreClient::connect_lazy(address, Duration::from_secs(10))?;
    client
        .add_book(AddBookRequest {
            ticker: ticker.clone(),
            kind: ContractKind::Binary as i32,
            tick_size: 1,
            min_price_ticks: 1,
            max_price_ticks: 99,
        })
        .await
        .ok();
    let gate = Arc::new(Semaphore::new(concurrency));
    let mut tasks = Vec::with_capacity(orders);
    for index in 0..orders {
        let permit = gate.clone().acquire_owned().await?;
        let client = client.clone();
        let ticker = ticker.clone();
        tasks.push(tokio::spawn(async move {
            let started = Instant::now();
            let result = client
                .submit_order(MeSubmitOrderRequest {
                    order_id: format!("load-{index}"),
                    user_id: format!("load-user-{}", index % 100),
                    hold_id: format!("load-hold-{index}"),
                    ticker,
                    side: Side::Yes as i32,
                    action: Action::Buy as i32,
                    price_ticks: 1 + (index as i64 % 10),
                    count: 1,
                    flags: 0,
                    stp: 0,
                })
                .await;
            drop(permit);
            (started.elapsed(), result.is_ok())
        }));
    }
    let mut latencies = Vec::with_capacity(orders);
    let mut succeeded = 0;
    for task in tasks {
        let (latency, ok) = task.await.context("load task panicked")?;
        latencies.push(latency);
        succeeded += usize::from(ok);
    }
    latencies.sort_unstable();
    let percentile =
        |p: usize| -> Duration { latencies[(latencies.len().saturating_sub(1) * p) / 100] };
    println!("{{\"orders\":{},\"succeeded\":{},\"concurrency\":{},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3}}}", orders, succeeded, concurrency, percentile(50).as_secs_f64() * 1000.0, percentile(95).as_secs_f64() * 1000.0, percentile(99).as_secs_f64() * 1000.0);
    Ok(())
}
