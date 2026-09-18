#[tokio::main]
async fn main() -> anyhow::Result<()> {
    sarvex_runtime::run_health_service("position-svc").await
}
