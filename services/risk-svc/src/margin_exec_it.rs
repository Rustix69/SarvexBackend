//! Integration test for cross margin execution against a real Postgres and a
//! running ledger-svc. Ignored by default.
//!
//! ```text
//! DATABASE_URL=postgres://... LEDGER_ADDR=http://127.0.0.1:50052 \
//!   cargo test -p risk-svc margin_exec_it -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Walks the design's CPI hedge example (a long future hedged by a short strip
//! binary) through reserve, fill, release, replay, insufficient funds, concurrency
//! and reconciliation, and checks the ledger invariants after each stage.

use crate::margin_exec::margin_account;
use crate::RiskService;
use sarvex_contracts::sarvex::v1::{
    ledger_client::LedgerClient, ref_data_client::RefDataClient, Action, AdminCreditDepositRequest,
    ApplyFillRequest, LedgerEntry, PostTransactionRequest, ReleaseOrderRequest,
    ReserveOrderRequest, Side,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tonic::transport::Endpoint;

const USDC: i64 = 1_000_000;
const EVENT: &str = "XEV-IT";
const FUTURE: &str = "SXF-IT";
const BINARY: &str = "SX-IT-T450";

async fn service() -> RiskService {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let ledger = std::env::var("LEDGER_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50052".into());
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&url)
        .await
        .expect("database");
    RiskService {
        pool,
        refdata: RefDataClient::new(
            Endpoint::from_static("http://127.0.0.1:1").connect_lazy(),
        ),
        ledger: LedgerClient::new(Endpoint::from_shared(ledger).unwrap().connect_lazy()),
        portfolio_enabled: true,
    }
}

async fn seed(pool: &PgPool) {
    for sql in [
        "INSERT INTO refdata.series (series_ticker, title) VALUES ('XS-IT','it') ON CONFLICT DO NOTHING",
        "INSERT INTO refdata.events (event_ticker, series_ticker, title, expected_resolution_at, margin_model, outcome_kind, outcome_divider) VALUES ('XEV-IT','XS-IT','it','2026-10-14T12:30:00Z','PORTFOLIO','NUMERIC',100) ON CONFLICT DO NOTHING",
        "INSERT INTO refdata.contracts (ticker, event_ticker, series_ticker, kind, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, tick_value_micro, divider, max_order_size, position_limit_per_user, state, expected_resolution_at, settlement_source, settlement_rule) VALUES ('SXF-IT','XEV-IT','XS-IT','SCALAR',1,400,600,400,600,10000000,100,1000,100000,'OPEN','2026-10-14T12:30:00Z','BLS CPI-U NSA, first release','{\"rounding_step\":1}') ON CONFLICT DO NOTHING",
        "INSERT INTO refdata.contracts (ticker, event_ticker, series_ticker, kind, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, tick_value_micro, divider, max_order_size, position_limit_per_user, state, expected_resolution_at, settlement_source, settlement_rule, payoff_rule, payout_ticks) VALUES ('SX-IT-T450','XEV-IT','XS-IT','BINARY',1,1,99,1,99,0,100,100000,100000,'OPEN','2026-10-14T12:30:00Z','BLS CPI-U NSA, first release','{\"rounding_step\":1}','{\"type\":\"range\",\"lo\":451,\"hi\":null,\"outcome_divider\":100}',100) ON CONFLICT DO NOTHING",
    ] {
        sqlx::query(sql).execute(pool).await.expect(sql);
    }
}

async fn fund(svc: &RiskService, user: &str, usdc: i64) {
    svc.ledger
        .clone()
        .admin_credit_deposit(AdminCreditDepositRequest {
            user_id: user.to_owned(),
            amount_micro_usdc: usdc * USDC,
            note: "it".into(),
        })
        .await
        .expect("deposit");
}

async fn account_balance(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar("SELECT COALESCE((SELECT e.running_balance_micro_usdc FROM ledger.entries e JOIN ledger.accounts a ON a.account_id=e.account_id WHERE a.account_code=$1 ORDER BY e.entry_id DESC LIMIT 1),0)::BIGINT")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// (cash, margin) in whole USDC.
async fn balances(svc: &RiskService, user: &str) -> (i64, i64) {
    let cash = account_balance(&svc.pool, &format!("LIAB:USER:{user}:CASH")).await;
    let margin = account_balance(&svc.pool, &margin_account(user, EVENT)).await;
    (cash / USDC, margin / USDC)
}

fn reserve(user: &str, order: &str, ticker: &str, side: Side, action: Action, price: i64, count: i64) -> ReserveOrderRequest {
    ReserveOrderRequest {
        user_id: user.into(),
        order_id: order.into(),
        ticker: ticker.into(),
        side: side as i32,
        action: action as i32,
        price_ticks: price,
        count,
        idempotency_key: format!("it:{order}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn fill(
    id: &str,
    seq: u64,
    ticker: &str,
    maker: (&str, &str, Side, Action),
    taker: (&str, &str, Side, Action),
    price: i64,
    count: i64,
) -> ApplyFillRequest {
    ApplyFillRequest {
        fill_id: id.into(),
        global_seq: seq,
        ticker: ticker.into(),
        maker_user_id: maker.0.into(),
        maker_order_id: maker.1.into(),
        taker_user_id: taker.0.into(),
        taker_order_id: taker.1.into(),
        maker_side: maker.2 as i32,
        maker_action: maker.3 as i32,
        taker_side: taker.2 as i32,
        taker_action: taker.3 as i32,
        price_ticks: price,
        count,
    }
}

async fn assert_invariants(svc: &RiskService, stage: &str) {
    let violations = svc.margin_invariants().await.expect("invariants");
    assert!(violations.is_empty(), "{stage}: {violations:?}");
}

#[tokio::test]
#[ignore]
async fn cpi_hedge_flows_through_reserve_fill_release_and_reconcile() {
    let svc = service().await;
    seed(&svc.pool).await;
    // Fresh users each run, so a rerun against the same database still works.
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let (alice, bob, carol, dave) = (
        format!("alice{run}"),
        format!("bob{run}"),
        format!("carol{run}"),
        format!("dave{run}"),
    );
    fund(&svc, &alice, 2000).await;
    fund(&svc, &bob, 5000).await;
    fund(&svc, &carol, 5000).await;
    fund(&svc, &dave, 100).await;

    // 1. Alice bids for one future at 5.00%: a long pays its premium, 1,000 USDC.
    let d = svc
        .reserve_order_impl(reserve(&alice, &format!("a1-{run}"), FUTURE, Side::Long, Action::Buy, 500, 1))
        .await
        .unwrap();
    assert!(d.approved, "{d:?}");
    assert_eq!(d.margin_after_micro_usdc, 1000 * USDC);
    assert_eq!(balances(&svc, &alice).await, (1000, 1000));

    // 1b. A retry of the same order changes nothing.
    let again = svc
        .reserve_order_impl(reserve(&alice, &format!("a1-{run}"), FUTURE, Side::Long, Action::Buy, 500, 1))
        .await
        .unwrap();
    assert!(again.approved);
    assert_eq!(balances(&svc, &alice).await, (1000, 1000));

    // 2. She also offers 500 YES at 80c ("CPI > 4.50%"): a hedge, so it needs no new margin.
    let d = svc
        .reserve_order_impl(reserve(&alice, &format!("a2-{run}"), BINARY, Side::Yes, Action::Sell, 80, 500))
        .await
        .unwrap();
    assert!(d.approved, "{d:?}");
    assert_eq!(d.margin_after_micro_usdc, 1000 * USDC, "a hedging order must not add margin");
    assert_eq!(balances(&svc, &alice).await, (1000, 1000));

    // 3. Counterparties rest their orders.
    let d = svc
        .reserve_order_impl(reserve(&bob, &format!("b1-{run}"), FUTURE, Side::Long, Action::Sell, 500, 1))
        .await
        .unwrap();
    assert_eq!(d.margin_after_micro_usdc, 1000 * USDC, "a short at 5.00% in a 4-6 band locks U - p");
    let d = svc
        .reserve_order_impl(reserve(&carol, &format!("c1-{run}"), BINARY, Side::Yes, Action::Buy, 80, 500))
        .await
        .unwrap();
    assert_eq!(d.margin_after_micro_usdc, 400 * USDC);
    assert_invariants(&svc, "after reserves").await;

    // 4. The future trades: bob is the maker, alice the taker.
    let f1 = fill(
        &format!("f1-{run}"),
        1,
        FUTURE,
        (&bob, &format!("b1-{run}"), Side::Long, Action::Sell),
        (&alice, &format!("a1-{run}"), Side::Long, Action::Buy),
        500,
        1,
    );
    let r = svc.apply_fill_impl(f1.clone()).await.unwrap();
    assert!(r.applied && !r.duplicate, "{r:?}");
    // Alice paid 1,000 out of margin; her margin is now 0 (the long cannot owe anything).
    assert_eq!(balances(&svc, &alice).await, (1000, 0));
    // Bob received 1,000 and must now post U - L = 2,000 for the short.
    assert_eq!(balances(&svc, &bob).await, (4000, 2000));
    assert_invariants(&svc, "after future fill").await;

    // 4b. A replay is a no-op.
    let r = svc.apply_fill_impl(f1).await.unwrap();
    assert!(r.duplicate && !r.applied);
    assert_eq!(balances(&svc, &bob).await, (4000, 2000));

    // 5. The strip binary trades: carol maker, alice taker.
    let f2 = fill(
        &format!("f2-{run}"),
        2,
        BINARY,
        (&carol, &format!("c1-{run}"), Side::Yes, Action::Buy),
        (&alice, &format!("a2-{run}"), Side::Yes, Action::Sell),
        80,
        500,
    );
    svc.apply_fill_impl(f2).await.unwrap();
    // Alice received 400: her hedged book now ties up 600 in total, as in the design.
    assert_eq!(balances(&svc, &alice).await, (1400, 0));
    assert_eq!(balances(&svc, &carol).await, (4600, 0));
    assert_invariants(&svc, "after binary fill").await;
    let margin = svc.get_account_margin_for_test(&alice).await;
    assert_eq!(margin, 0);

    // 6. Insufficient funds: dave has 100 USDC and bids 1,000 USDC of premium.
    let d = svc
        .reserve_order_impl(reserve(&dave, &format!("d1-{run}"), FUTURE, Side::Long, Action::Buy, 500, 1))
        .await
        .unwrap();
    assert!(!d.approved);
    assert_eq!(d.reject_code, "INSUFFICIENT_MARGIN");
    assert_eq!(balances(&svc, &dave).await, (100, 0));
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM risk.margin_orders WHERE user_id=$1")
        .bind(&dave)
        .fetch_one(&svc.pool)
        .await
        .unwrap();
    assert_eq!(stored, 0, "a rejected order must leave no trace");

    // 7. A resting order raises bob's margin; releasing it gives the cash back.
    let d = svc
        .reserve_order_impl(reserve(&bob, &format!("b2-{run}"), FUTURE, Side::Long, Action::Sell, 550, 1))
        .await
        .unwrap();
    assert_eq!(d.margin_after_micro_usdc, 2500 * USDC);
    assert_eq!(balances(&svc, &bob).await, (3500, 2500));
    let d = svc
        .release_order_impl(ReleaseOrderRequest {
            user_id: bob.clone(),
            order_id: format!("b2-{run}"),
            remaining_count: 0,
            idempotency_key: String::new(),
            global_seq: 3,
            reason: "CANCEL".into(),
        })
        .await
        .unwrap();
    assert!(d.approved);
    assert_eq!(balances(&svc, &bob).await, (4000, 2000));
    // releasing twice is harmless
    let d = svc
        .release_order_impl(ReleaseOrderRequest {
            user_id: bob.clone(),
            order_id: format!("b2-{run}"),
            remaining_count: 0,
            idempotency_key: String::new(),
            global_seq: 3,
            reason: "CANCEL".into(),
        })
        .await
        .unwrap();
    assert!(d.approved);
    assert_eq!(balances(&svc, &bob).await, (4000, 2000));
    assert_invariants(&svc, "after release").await;

    // 8. Reconciliation: someone moves 7 USDC into alice's margin account behind risk-svc's back.
    svc.ledger
        .clone()
        .post_transaction(PostTransactionRequest {
            idempotency_key: format!("it:drift:{run}"),
            reason_code: "TEST_DRIFT".into(),
            entries: vec![
                LedgerEntry { account_code: format!("LIAB:USER:{alice}:CASH"), direction: "DR".into(), amount_micro_usdc: 7 * USDC, memo: String::new() },
                LedgerEntry { account_code: margin_account(&alice, EVENT), direction: "CR".into(), amount_micro_usdc: 7 * USDC, memo: String::new() },
            ],
            metadata: None,
        })
        .await
        .unwrap();
    assert!(!svc.margin_invariants().await.unwrap().is_empty(), "drift must be visible");
    let mut suspects = std::collections::HashMap::new();
    let first = svc.reconcile_margin(&mut suspects).await.unwrap();
    assert!(first.iter().any(|d| d.user_id == alice), "first pass notices the drift");
    assert_eq!(balances(&svc, &alice).await, (1393, 7), "first pass only flags it");
    svc.reconcile_margin(&mut suspects).await.unwrap();
    assert_eq!(balances(&svc, &alice).await, (1400, 0), "second pass corrects it");
    assert_invariants(&svc, "after reconcile").await;

    // 9. Concurrency: eight simultaneous 1,000 USDC bids against 3,000 USDC approve exactly three.
    let eve = format!("eve{run}");
    fund(&svc, &eve, 3000).await;
    let mut handles = Vec::new();
    for i in 0..8 {
        let svc = svc.clone();
        let eve = eve.clone();
        handles.push(tokio::spawn(async move {
            svc.reserve_order_impl(reserve(&eve, &format!("e{i}-{run}"), FUTURE, Side::Long, Action::Buy, 500, 1))
                .await
                .unwrap()
        }));
    }
    let mut approved = 0;
    for handle in handles {
        if handle.await.unwrap().approved {
            approved += 1;
        }
    }
    assert_eq!(approved, 3);
    assert_eq!(balances(&svc, &eve).await, (0, 3000));
    assert_invariants(&svc, "after concurrency").await;
}

impl RiskService {
    /// Total margin from the margin tables, via the public RPC path.
    async fn get_account_margin_for_test(&self, user: &str) -> i64 {
        use sarvex_contracts::sarvex::v1::{risk_server::Risk, GetAccountMarginRequest};
        let response = self
            .get_account_margin(tonic::Request::new(GetAccountMarginRequest {
                user_id: user.to_owned(),
            }))
            .await
            .expect("get_account_margin")
            .into_inner();
        assert_eq!(response.mode, "LIVE");
        response.total_margin_micro_usdc / USDC
    }
}
