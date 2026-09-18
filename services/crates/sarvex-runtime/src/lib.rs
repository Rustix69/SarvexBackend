use anyhow::Result;
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Router};
use serde_json::json;
use std::{env, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[derive(Clone)]
struct HealthState {
    service: Arc<str>,
    require_db: bool,
}

pub fn init_tracing(service: &str) {
    let filter = env::var("RUST_LOG").unwrap_or_else(|_| "info".to_owned());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
    tracing::info!(service, "service_starting");
}

pub async fn run_health_service(service: &str) -> Result<()> {
    init_tracing(service);
    let port = env::var("HTTP_PORT").unwrap_or_else(|_| "8080".to_owned());
    let require_db = env::var("REQUIRE_DB").map(|v| v == "true").unwrap_or(false);
    let state = HealthState {
        service: Arc::from(service.to_owned()),
        require_db,
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn healthz(State(state): State<HealthState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(json!({ "status": "ok", "service": state.service })),
    )
}

async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    if state.require_db {
        // Domain services override this endpoint with a real database probe.
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({ "status": "not_ready", "reason": "database_probe_required" })),
        );
    }
    (
        StatusCode::OK,
        axum::Json(json!({ "status": "ready", "service": state.service })),
    )
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
    tokio::time::sleep(Duration::from_millis(10)).await;
}
