//! Integration test for the shadow comparison log against a real Postgres. Ignored by default.
//!
//! ```text
//! DATABASE_URL=postgres://... cargo test -p risk-svc shadow_it -- --ignored --nocapture
//! ```
//!
//! Seeds an ISOLATED event with two binaries, a bull spread held by one account (and its
//! mirror by the counterparty) plus a working order, runs one pass and checks the rows.

use crate::RiskService;
use sarvex_contracts::sarvex::v1::{ledger_client::LedgerClient, ref_data_client::RefDataClient};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use tonic::transport::Endpoint;

const EVENT: &str = "XEV-SH";
const LOW: &str = "SX-SH-T340";
const HIGH: &str = "SX-SH-T360";

async fn service() -> RiskService {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("database");
    RiskService {
        pool,
        refdata: RefDataClient::new(Endpoint::from_static("http://127.0.0.1:1").connect_lazy()),
        ledger: LedgerClient::new(Endpoint::from_static("http://127.0.0.1:1").connect_lazy()),
        portfolio_enabled: false,
    }
}

async fn seed(pool: &PgPool) {
    let binary = |ticker: &str, lo: i64| {
        format!(
            "INSERT INTO refdata.contracts (ticker, event_ticker, series_ticker, kind, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, tick_value_micro, divider, max_order_size, position_limit_per_user, state, expected_resolution_at, settlement_source, settlement_rule, payoff_rule, payout_ticks) VALUES ('{ticker}','{EVENT}','XS-SH','BINARY',1,1,99,1,99,0,100,100000,100000,'OPEN','2026-10-14T12:30:00Z','BLS CPI-U NSA, first release','{{\"rounding_step\":1}}','{{\"type\":\"range\",\"lo\":{lo},\"hi\":null,\"outcome_divider\":100}}',100) ON CONFLICT DO NOTHING"
        )
    };
    for sql in [
        "INSERT INTO refdata.series (series_ticker, title) VALUES ('XS-SH','sh') ON CONFLICT DO NOTHING".to_owned(),
        format!("INSERT INTO refdata.events (event_ticker, series_ticker, title, expected_resolution_at, margin_model, outcome_kind, outcome_divider) VALUES ('{EVENT}','XS-SH','sh','2026-10-14T12:30:00Z','ISOLATED','NUMERIC',100) ON CONFLICT DO NOTHING"),
        binary(LOW, 341),
        binary(HIGH, 361),
    ] {
        sqlx::query(&sql).execute(pool).await.expect(&sql);
    }
}

#[allow(clippy::too_many_arguments)]
async fn order(
    pool: &PgPool,
    id: &str,
    user: &str,
    ticker: &str,
    action: i32,
    price: i64,
    count: i64,
    filled: i64,
    status: &str,
    hold: i64,
) {
    sqlx::query("INSERT INTO orders.orders (order_id, user_id, client_order_id, ticker, side, action, price_ticks, count, filled_count, tif, status, hold_amount_micro_usdc) VALUES ($1,$2,$1,$3,1,$4,$5,$6,$7,1,$8,$9)")
        .bind(id).bind(user).bind(ticker).bind(action).bind(price).bind(count).bind(filled).bind(status).bind(hold)
        .execute(pool).await.expect("order");
}

#[allow(clippy::too_many_arguments)]
async fn fill(
    pool: &PgPool,
    id: &str,
    seq: i64,
    ticker: &str,
    maker: (&str, &str, i32),
    taker: (&str, &str, i32),
    price: i64,
    count: i64,
) {
    sqlx::query("INSERT INTO orders.fills (fill_id, ticker, global_seq, contract_seq, maker_order_id, taker_order_id, maker_user_id, taker_user_id, maker_side, maker_action, taker_side, taker_action, price_ticks, count, aggressor_side) VALUES ($1,$2,$3,$3,$4,$5,$6,$7,1,$8,1,$9,$10,$11,1)")
        .bind(id).bind(ticker).bind(seq).bind(maker.1).bind(taker.1).bind(maker.0).bind(taker.0).bind(maker.2).bind(taker.2).bind(price).bind(count)
        .execute(pool).await.expect("fill");
}

async fn position(pool: &PgPool, user: &str, ticker: &str, qty: i64) {
    sqlx::query("INSERT INTO position.positions (user_id, ticker, net_qty) VALUES ($1,$2,$3)")
        .bind(user)
        .bind(ticker)
        .bind(qty)
        .execute(pool)
        .await
        .expect("position");
}

#[tokio::test]
#[ignore]
async fn shadow_pass_logs_a_bull_spread_below_todays_capital() {
    let svc = service().await;
    seed(&svc.pool).await;
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (a, b) = (format!("sha{run}"), format!("shb{run}"));
    let seq = 8_000_000_000_i64 + (run % 1_000_000_000) as i64 * 10;
    // Action ids: BUY = 1, SELL = 2 (proto enum).
    let (buy, sell) = (1, 2);
    // A buys 10 of T340 at 60 from B, then sells 10 of T360 at 25 to B.
    order(
        &svc.pool,
        &format!("{a}-o1"),
        &a,
        LOW,
        buy,
        60,
        10,
        10,
        "FILLED",
        0,
    )
    .await;
    order(
        &svc.pool,
        &format!("{b}-o1"),
        &b,
        LOW,
        sell,
        60,
        10,
        10,
        "FILLED",
        0,
    )
    .await;
    order(
        &svc.pool,
        &format!("{a}-o2"),
        &a,
        HIGH,
        sell,
        25,
        10,
        10,
        "FILLED",
        0,
    )
    .await;
    order(
        &svc.pool,
        &format!("{b}-o2"),
        &b,
        HIGH,
        buy,
        25,
        10,
        10,
        "FILLED",
        0,
    )
    .await;
    fill(
        &svc.pool,
        &format!("{a}-f1"),
        seq,
        LOW,
        (&b, &format!("{b}-o1"), sell),
        (&a, &format!("{a}-o1"), buy),
        60,
        10,
    )
    .await;
    fill(
        &svc.pool,
        &format!("{a}-f2"),
        seq + 1,
        HIGH,
        (&b, &format!("{b}-o2"), buy),
        (&a, &format!("{a}-o2"), sell),
        25,
        10,
    )
    .await;
    position(&svc.pool, &a, LOW, 10).await;
    position(&svc.pool, &a, HIGH, -10).await;
    position(&svc.pool, &b, LOW, -10).await;
    position(&svc.pool, &b, HIGH, 10).await;
    // A also works a buy of 5 T340 at 50, holding 2.5 USDC.
    order(
        &svc.pool,
        &format!("{a}-o3"),
        &a,
        LOW,
        buy,
        50,
        5,
        0,
        "OPEN",
        2_500_000,
    )
    .await;

    let summary = svc.shadow_pass().await.expect("pass");
    assert!(summary.logged >= 2, "{summary:?}");

    let rows = sqlx::query("SELECT user_id, net_premium_micro_usdc, margin_micro_usdc, capital_cross_micro_usdc, capital_today_micro_usdc, worst_case_micro_usdc, strategy_tag, violation FROM risk.margin_shadow_log WHERE event_ticker=$1 AND user_id = ANY($2) ORDER BY user_id")
        .bind(EVENT)
        .bind(vec![a.clone(), b.clone()])
        .fetch_all(&svc.pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let row_a = &rows[0];
    assert_eq!(row_a.get::<String, _>("user_id"), a);
    assert_eq!(row_a.get::<i64, _>("net_premium_micro_usdc"), 3_500_000);
    // Long 6.00 + short collateral 7.50 + 2.50 held for the order.
    assert_eq!(row_a.get::<i64, _>("capital_today_micro_usdc"), 16_000_000);
    assert_eq!(row_a.get::<Option<String>, _>("violation"), None);
    assert!(row_a.get::<i64, _>("capital_cross_micro_usdc") <= 16_000_000);
    assert!(
        row_a.get::<i64, _>("worst_case_micro_usdc")
            <= row_a.get::<i64, _>("capital_cross_micro_usdc")
    );
    let row_b = &rows[1];
    assert_eq!(row_b.get::<String, _>("user_id"), b);
    assert_eq!(row_b.get::<i64, _>("net_premium_micro_usdc"), -3_500_000);
    assert_eq!(row_b.get::<Option<String>, _>("violation"), None);

    // A second pass appends rather than updating, and nothing violates.
    svc.shadow_pass().await.expect("second pass");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM risk.margin_shadow_log WHERE event_ticker=$1 AND user_id=$2",
    )
    .bind(EVENT)
    .bind(&a)
    .fetch_one(&svc.pool)
    .await
    .unwrap();
    assert_eq!(count, 2);
}
