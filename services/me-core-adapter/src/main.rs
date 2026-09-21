use sarvex_me_client::MeCoreClient;
use std::{env, time::Duration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address = env::var("ME_CORE_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50054".to_owned());
    let timeout_ms = env::var("ME_CORE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(100);
    let _client = MeCoreClient::connect_lazy(address.clone(), Duration::from_millis(timeout_ms))?;
    tracing::info!(me_core_addr = %address, timeout_ms, "me-core client boundary initialized");
    sarvex_runtime::run_health_service("me-core-adapter").await
}
