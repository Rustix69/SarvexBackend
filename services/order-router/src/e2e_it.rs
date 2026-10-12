//! End-to-end test of cross margin through the real order-router, me-core, risk-svc,
//! ledger-svc and refdata-svc. Ignored by default; see `scratchpad/it.sh stack`.
//!
//! ```text
//! DATABASE_URL=... ROUTER_ADDR=http://mt-router:50055 LEDGER_ADDR=http://mt-ledger:50052 \
//!   cargo test -p order-router e2e_it -- --ignored --nocapture --test-threads=1
//! ```

use sarvex_contracts::sarvex::v1::{
    ledger_client::LedgerClient, order_router_client::OrderRouterClient, Action,
    AdminCreditDepositRequest, CancelOrderRequest, Side, SubmitOrderRequest, SubmitOrderResponse,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};

const USDC: i64 = 1_000_000;

async fn balance(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar("SELECT COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code=$1 ORDER BY e.entry_id DESC LIMIT 1),0)::BIGINT")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// (cash, margin) in whole USDC.
async fn balances(pool: &PgPool, user: &str, event: &str) -> (i64, i64) {
    (
        balance(pool, &format!("LIAB:USER:{user}:CASH")).await / USDC,
        balance(pool, &format!("LIAB:USER:{user}:MARGIN:{event}")).await / USDC,
    )
}

async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
    for _ in 0..60 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("timed out waiting for: {what}");
}

#[allow(clippy::too_many_arguments)]
async fn submit(
    router: &mut OrderRouterClient<Channel>,
    user: &str,
    client_id: &str,
    ticker: &str,
    side: Side,
    action: Action,
    price: i64,
    count: i64,
    tif: i32,
) -> SubmitOrderResponse {
    router
        .submit_order(SubmitOrderRequest {
            user_id: user.into(),
            client_order_id: client_id.into(),
            ticker: ticker.into(),
            side: side as i32,
            action: action as i32,
            price_ticks: price,
            count,
            tif,
            idempotency_key: client_id.into(),
            ..Default::default()
        })
        .await
        .expect("submit_order")
        .into_inner()
}

#[tokio::test]
#[ignore]
async fn cpi_hedge_through_the_whole_stack() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let ledger = LedgerClient::new(
        Endpoint::from_shared(std::env::var("LEDGER_ADDR").unwrap())
            .unwrap()
            .connect_lazy(),
    );
    let mut router = OrderRouterClient::connect(std::env::var("ROUTER_ADDR").unwrap())
        .await
        .unwrap();
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let event = format!("XEV-E{run}");
    let future = format!("SXF-E{run}");
    let binary = format!("SX-E{run}-T450");
    for sql in [
        "INSERT INTO refdata.series (series_ticker, title) VALUES ('XS-E2E','e2e') ON CONFLICT DO NOTHING".to_owned(),
        format!("INSERT INTO refdata.events (event_ticker, series_ticker, title, expected_resolution_at, margin_model, outcome_kind, outcome_divider) VALUES ('{event}','XS-E2E','e2e','2026-10-14T12:30:00Z','PORTFOLIO','NUMERIC',100)"),
        format!("INSERT INTO refdata.contracts (ticker, event_ticker, series_ticker, kind, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, tick_value_micro, divider, max_order_size, position_limit_per_user, state, expected_resolution_at, settlement_source, settlement_rule) VALUES ('{future}','{event}','XS-E2E','SCALAR',1,400,600,400,600,10000000,100,100000,100000,'OPEN','2026-10-14T12:30:00Z','BLS','{{\"rounding_step\":1}}')"),
        format!("INSERT INTO refdata.contracts (ticker, event_ticker, series_ticker, kind, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, tick_value_micro, divider, max_order_size, position_limit_per_user, state, expected_resolution_at, settlement_source, settlement_rule, payoff_rule, payout_ticks) VALUES ('{binary}','{event}','XS-E2E','BINARY',1,1,99,1,99,0,100,100000,100000,'OPEN','2026-10-14T12:30:00Z','BLS','{{\"rounding_step\":1}}','{{\"type\":\"range\",\"lo\":451,\"hi\":null,\"outcome_divider\":100}}',100)"),
    ] {
        sqlx::query(&sql).execute(&pool).await.expect(&sql);
    }
    let users: Vec<(String, i64)> = [("alice", 2000), ("bob", 5000), ("carol", 5000), ("dave", 100), ("erin", 2000)]
        .into_iter()
        .map(|(name, usdc)| (format!("{name}{run}"), usdc))
        .collect();
    for (user, usdc) in &users {
        sqlx::query("INSERT INTO risk.user_limits (user_id, kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc) VALUES ($1,1,100000000000000,100000000000000) ON CONFLICT DO NOTHING")
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
        ledger
            .clone()
            .admin_credit_deposit(AdminCreditDepositRequest {
                user_id: user.clone(),
                amount_micro_usdc: usdc * USDC,
                note: "e2e".into(),
            })
            .await
            .unwrap();
    }
    let (alice, bob, carol, dave, erin) = (&users[0].0, &users[1].0, &users[2].0, &users[3].0, &users[4].0);

    // 1. Bob offers one future at 5.00%: a short in a 4-6 band locks U - p = 1,000 USDC.
    let r = submit(&mut router, bob, &format!("b1-{run}"), &future, Side::Long, Action::Sell, 500, 1, 1).await;
    assert_eq!(r.reject_code, "", "{r:?}");
    assert_eq!(balances(&pool, bob, &event).await, (4000, 1000));

    // 2. Alice lifts it: she pays 1,000 premium, bob must now post 2,000.
    let r = submit(&mut router, alice, &format!("a1-{run}"), &future, Side::Long, Action::Buy, 500, 1, 1).await;
    assert_eq!(r.reject_code, "", "{r:?}");
    assert_eq!(r.fills.len(), 1);
    eventually("future fill applied to margin", async || {
        balances(&pool, alice, &event).await == (1000, 0) && balances(&pool, bob, &event).await == (4000, 2000)
    })
    .await;

    // 3. Alice hedges by offering 500 YES at 80c; it needs no margin. Carol takes it.
    let r = submit(&mut router, alice, &format!("a2-{run}"), &binary, Side::Yes, Action::Sell, 80, 500, 1).await;
    assert_eq!(r.reject_code, "", "{r:?}");
    assert_eq!(balances(&pool, alice, &event).await, (1000, 0), "a hedging order adds no margin");
    let r = submit(&mut router, carol, &format!("c1-{run}"), &binary, Side::Yes, Action::Buy, 80, 500, 1).await;
    assert_eq!(r.reject_code, "", "{r:?}");
    assert_eq!(r.fills.len(), 1);
    eventually("binary fill applied to margin", async || {
        balances(&pool, alice, &event).await == (1400, 0) && balances(&pool, carol, &event).await == (4600, 0)
    })
    .await;

    // 4. A resting order raises bob's margin; cancelling it gives the cash back.
    let r = submit(&mut router, bob, &format!("b2-{run}"), &future, Side::Long, Action::Sell, 550, 1, 1).await;
    assert_eq!(r.reject_code, "", "{r:?}");
    let order_id = r.order.unwrap().order_id;
    assert_eq!(balances(&pool, bob, &event).await, (3500, 2500));
    router
        .cancel_order(CancelOrderRequest {
            user_id: bob.clone(),
            order_id,
            client_order_id: String::new(),
        })
        .await
        .unwrap();
    eventually("cancelled order margin released", async || {
        balances(&pool, bob, &event).await == (4000, 2000)
    })
    .await;

    // 5. Not enough cash: rejected with INSUFFICIENT_MARGIN and nothing is held.
    let r = submit(&mut router, dave, &format!("d1-{run}"), &future, Side::Long, Action::Buy, 500, 1, 1).await;
    assert_eq!(r.reject_code, "INSUFFICIENT_MARGIN", "{r:?}");
    assert_eq!(balances(&pool, dave, &event).await, (100, 0));

    // 6. An IOC that finds nothing is cancelled and its margin comes back.
    let r = submit(&mut router, erin, &format!("e1-{run}"), &future, Side::Long, Action::Buy, 450, 1, 2).await;
    assert_eq!(r.reject_code, "", "{r:?}");
    eventually("IOC margin released", async || {
        balances(&pool, erin, &event).await == (2000, 0)
    })
    .await;

    // 7. Ledger and risk agree, CLEARING is empty, open interest nets to zero.
    let clearing = balance(&pool, &format!("LIAB:HOUSE:CLEARING:{event}")).await;
    assert_eq!(clearing, 0);
    let drift: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM risk.event_margin em WHERE em.event_ticker=$1 AND em.posted_micro_usdc <> COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code='LIAB:USER:'||em.user_id||':MARGIN:'||em.event_ticker ORDER BY e.entry_id DESC LIMIT 1),0)")
        .bind(&event)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(drift, 0);
    let orders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM risk.margin_orders WHERE event_ticker=$1")
        .bind(&event)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(orders, 0, "every finished order's margin must be gone");
    let unbalanced: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM (SELECT ticker FROM risk.margin_positions WHERE event_ticker=$1 GROUP BY ticker HAVING SUM(net_qty)<>0) t")
        .bind(&event)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unbalanced, 0);
}
