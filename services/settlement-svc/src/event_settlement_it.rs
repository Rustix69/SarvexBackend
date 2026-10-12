//! Integration test for `SettleEvent` against a real Postgres and ledger-svc. Ignored by default.
//!
//! ```text
//! DATABASE_URL=postgres://... LEDGER_ADDR=http://127.0.0.1:50052 \
//!   cargo test -p settlement-svc event_settlement_it -- --ignored --nocapture --test-threads=1
//! ```

use crate::event_settlement::settle_event;
use sarvex_contracts::sarvex::v1::{
    ledger_client::LedgerClient, AdminCreditDepositRequest, LedgerEntry, PostTransactionRequest,
};
use sarvex_domain::margin::Outcome;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tonic::transport::{Channel, Endpoint};

const USDC: i64 = 1_000_000;

async fn connect() -> (PgPool, LedgerClient<Channel>) {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let ledger = std::env::var("LEDGER_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50052".into());
    let pool = PgPoolOptions::new().max_connections(10).connect(&url).await.unwrap();
    let ledger = LedgerClient::new(Endpoint::from_shared(ledger).unwrap().connect_lazy());
    (pool, ledger)
}

async fn balance(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar("SELECT COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code=$1 ORDER BY e.entry_id DESC LIMIT 1),0)::BIGINT")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn cash(pool: &PgPool, user: &str) -> i64 {
    balance(pool, &format!("LIAB:USER:{user}:CASH")).await / USDC
}

async fn margin(pool: &PgPool, user: &str, event: &str) -> i64 {
    balance(pool, &format!("LIAB:USER:{user}:MARGIN:{event}")).await / USDC
}

async fn seed_event(pool: &PgPool, event: &str) -> (String, String) {
    let future = format!("{event}-F");
    let binary = format!("{event}-B450");
    sqlx::query("INSERT INTO refdata.series (series_ticker, title) VALUES ('XS-ST','st') ON CONFLICT DO NOTHING")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO refdata.events (event_ticker, series_ticker, title, expected_resolution_at, margin_model, outcome_kind, outcome_divider) VALUES ($1,'XS-ST','st','2026-10-14T12:30:00Z','PORTFOLIO','NUMERIC',100)")
        .bind(event)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO refdata.contracts (ticker, event_ticker, series_ticker, kind, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, tick_value_micro, divider, max_order_size, position_limit_per_user, state, expected_resolution_at, settlement_source, settlement_rule) VALUES ($1,$2,'XS-ST','SCALAR',1,400,600,400,600,10000000,100,1000,100000,'CLOSED','2026-10-14T12:30:00Z','BLS','{\"rounding_step\":1}')")
        .bind(&future)
        .bind(event)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO refdata.contracts (ticker, event_ticker, series_ticker, kind, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, tick_value_micro, divider, max_order_size, position_limit_per_user, state, expected_resolution_at, settlement_source, settlement_rule, payoff_rule, payout_ticks) VALUES ($1,$2,'XS-ST','BINARY',1,1,99,1,99,0,100,100000,100000,'CLOSED','2026-10-14T12:30:00Z','BLS','{\"rounding_step\":1}','{\"type\":\"range\",\"lo\":451,\"hi\":null,\"outcome_divider\":100}',100)")
        .bind(&binary)
        .bind(event)
        .execute(pool)
        .await
        .unwrap();
    (future, binary)
}

async fn position(pool: &PgPool, user: &str, ticker: &str, event: &str, qty: i64) {
    sqlx::query("INSERT INTO risk.margin_positions (user_id, ticker, event_ticker, net_qty) VALUES ($1,$2,$3,$4)")
        .bind(user)
        .bind(ticker)
        .bind(event)
        .bind(qty)
        .execute(pool)
        .await
        .unwrap();
}

/// What the account holds after trading: margin posted on the ledger and in risk.event_margin.
async fn post_margin(pool: &PgPool, ledger: &LedgerClient<Channel>, user: &str, event: &str, usdc: i64) {
    ledger
        .clone()
        .post_transaction(PostTransactionRequest {
            idempotency_key: format!("st:margin:{user}:{event}"),
            reason_code: "TEST_MARGIN".into(),
            entries: vec![
                LedgerEntry { account_code: format!("LIAB:USER:{user}:CASH"), direction: "DR".into(), amount_micro_usdc: usdc * USDC, memo: String::new() },
                LedgerEntry { account_code: format!("LIAB:USER:{user}:MARGIN:{event}"), direction: "CR".into(), amount_micro_usdc: usdc * USDC, memo: String::new() },
            ],
            metadata: None,
        })
        .await
        .unwrap();
    sqlx::query("INSERT INTO risk.event_margin (user_id, event_ticker, margin_micro_usdc, position_requirement_micro_usdc, posted_micro_usdc, standalone_micro_usdc, version) VALUES ($1,$2,$3,$3,$3,$3,1)")
        .bind(user)
        .bind(event)
        .bind(usdc * USDC)
        .execute(pool)
        .await
        .unwrap();
}

async fn fund(ledger: &LedgerClient<Channel>, user: &str, usdc: i64) {
    ledger
        .clone()
        .admin_credit_deposit(AdminCreditDepositRequest {
            user_id: user.to_owned(),
            amount_micro_usdc: usdc * USDC,
            note: "st".into(),
        })
        .await
        .unwrap();
}

#[tokio::test]
#[ignore]
async fn events_settle_zero_sum_and_release_all_margin() {
    let (pool, ledger) = connect().await;
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (alice, bob, carol) = (format!("sa{run}"), format!("sb{run}"), format!("sc{run}"));
    fund(&ledger, &alice, 2000).await;
    fund(&ledger, &bob, 5000).await;
    fund(&ledger, &carol, 5000).await;

    // The book after the design's CPI trades: alice long 1 future and short 500 YES, bob short
    // 1 future (2,000 posted), carol long 500 YES.
    let event = format!("XEV-ST{run}");
    let (future, binary) = seed_event(&pool, &event).await;
    position(&pool, &alice, &future, &event, 1).await;
    position(&pool, &alice, &binary, &event, -500).await;
    position(&pool, &bob, &future, &event, -1).await;
    position(&pool, &carol, &binary, &event, 500).await;
    post_margin(&pool, &ledger, &bob, &event, 2000).await;
    let total_before = cash(&pool, &alice).await + cash(&pool, &bob).await + cash(&pool, &carol).await
        + margin(&pool, &bob, &event).await;

    // CPI prints 5.00%: the future pays 1,000, the strip binary resolves YES.
    let r = settle_event(&pool, &ledger, &event, Outcome::Numeric(500)).await.unwrap();
    assert_eq!(r.status, "COMPLETED");
    assert_eq!(r.accounts_settled, 3);
    assert_eq!(r.total_credits_micro_usdc, 1000 * USDC);
    assert_eq!(r.total_debits_micro_usdc, 1000 * USDC);
    // alice +1000 (future) -500 (short YES) = +500; bob -1000; carol +500 (long YES).
    assert_eq!(cash(&pool, &alice).await, 2500);
    assert_eq!(cash(&pool, &bob).await, 3000 + 2000 - 1000);
    assert_eq!(cash(&pool, &carol).await, 5500);
    assert_eq!(margin(&pool, &bob, &event).await, 0);
    assert_eq!(balance(&pool, &format!("LIAB:HOUSE:CLEARING:{event}")).await, 0);
    let total_after = cash(&pool, &alice).await + cash(&pool, &bob).await + cash(&pool, &carol).await;
    assert_eq!(total_before, total_after, "settlement must not create or destroy money");
    let state: String = sqlx::query_scalar("SELECT state::text FROM refdata.contracts WHERE ticker=$1")
        .bind(&future)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "SETTLED");
    let posted: i64 = sqlx::query_scalar("SELECT posted_micro_usdc FROM risk.event_margin WHERE user_id=$1 AND event_ticker=$2")
        .bind(&bob)
        .bind(&event)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(posted, 0, "risk state must agree with the ledger");

    // Settling again changes nothing.
    let again = settle_event(&pool, &ledger, &event, Outcome::Numeric(500)).await.unwrap();
    assert_eq!(again.status, "COMPLETED");
    assert_eq!(cash(&pool, &alice).await, 2500);

    // A second event, CPI at 4.20%: the future pays 200, the binary resolves NO.
    let event2 = format!("XEV-ST2{run}");
    let (future2, binary2) = seed_event(&pool, &event2).await;
    position(&pool, &alice, &future2, &event2, 1).await;
    position(&pool, &alice, &binary2, &event2, -500).await;
    position(&pool, &bob, &future2, &event2, -1).await;
    position(&pool, &carol, &binary2, &event2, 500).await;
    post_margin(&pool, &ledger, &bob, &event2, 2000).await;
    let r = settle_event(&pool, &ledger, &event2, Outcome::Numeric(420)).await.unwrap();
    assert_eq!(r.total_credits_micro_usdc, 200 * USDC);
    assert_eq!(cash(&pool, &alice).await, 2700);
    assert_eq!(cash(&pool, &bob).await, 4000 - 2000 + 2000 - 200);
    assert_eq!(cash(&pool, &carol).await, 5500, "NO won, so the long YES pays nothing and owes nothing");
    assert_eq!(balance(&pool, &format!("LIAB:HOUSE:CLEARING:{event2}")).await, 0);

    // An event whose open interest does not net to zero is refused and nothing moves.
    let event3 = format!("XEV-ST3{run}");
    let (future3, _) = seed_event(&pool, &event3).await;
    position(&pool, &alice, &future3, &event3, 1).await;
    let before = cash(&pool, &alice).await;
    let error = settle_event(&pool, &ledger, &event3, Outcome::Numeric(500)).await.unwrap_err();
    assert!(error.message().contains("open interest"), "{error}");
    assert_eq!(cash(&pool, &alice).await, before);

    // An outcome off the contract's possible values is refused too (a categorical on a numeric event).
    let error = settle_event(&pool, &ledger, &event3, Outcome::Category("HOLD".into()))
        .await
        .unwrap_err();
    assert!(!error.message().is_empty());
}
