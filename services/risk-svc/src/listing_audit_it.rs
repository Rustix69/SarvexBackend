//! Audit: runs the cross-margin listing validator (CM-06, CM-11) over every event in the
//! database and prints which pass. Ignored by default; read-only.
//!
//! ```text
//! DATABASE_URL=postgres://... cargo test -p risk-svc listing_audit_it -- --ignored --nocapture
//! ```

use crate::{portfolio, RiskService};
use sarvex_contracts::sarvex::v1::{ledger_client::LedgerClient, ref_data_client::RefDataClient};
use sqlx::postgres::PgPoolOptions;
use tonic::transport::Endpoint;

#[tokio::test]
#[ignore]
async fn every_event_listing_audit() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("database");
    let svc = RiskService {
        pool: pool.clone(),
        refdata: RefDataClient::new(Endpoint::from_static("http://127.0.0.1:1").connect_lazy()),
        ledger: LedgerClient::new(Endpoint::from_static("http://127.0.0.1:1").connect_lazy()),
        portfolio_enabled: false,
    };
    let tickers: Vec<String> =
        sqlx::query_scalar("SELECT event_ticker FROM refdata.events ORDER BY event_ticker")
            .fetch_all(&pool)
            .await
            .unwrap();
    let (events, contracts) = svc.load_event_refdata(&tickers).await.expect("refdata");
    let (mut ok, mut bad) = (0, 0);
    for ticker in &tickers {
        match portfolio::check_listing(&events[ticker], &contracts) {
            Ok(()) => {
                ok += 1;
                println!("PASS {ticker}");
            }
            Err(reason) => {
                bad += 1;
                println!("FAIL {ticker}: {}", reason);
            }
        }
    }
    println!("listing audit: {ok} pass, {bad} fail");
}
