use futures_util::StreamExt;
use sarvex_contracts::sarvex::v1::{
    matching_engine_client::MatchingEngineClient, Action, AddBookRequest, ContractKind,
    GetBookSnapshotRequest, MeSubmitOrderRequest, Side, StreamExecutionsRequest,
};
use std::env;
use tonic::Request;

#[tokio::test]
#[ignore = "requires the C++ me-core server; run with ME_CORE_TEST_ADDR"]
async fn rust_client_matches_against_cpp_me_core() {
    let address = env::var("ME_CORE_TEST_ADDR").expect("ME_CORE_TEST_ADDR");
    let mut client = MatchingEngineClient::connect(address)
        .await
        .expect("connect to me-core");
    let ticker = "SMOKE-BINARY";
    client
        .add_book(Request::new(AddBookRequest {
            ticker: ticker.to_owned(),
            kind: ContractKind::Binary as i32,
            tick_size: 1,
            min_price_ticks: 1,
            max_price_ticks: 99,
        }))
        .await
        .expect("add book");

    let maker = client
        .submit_order(Request::new(MeSubmitOrderRequest {
            order_id: "smoke-maker".to_owned(),
            user_id: "maker".to_owned(),
            hold_id: "hold-maker".to_owned(),
            ticker: ticker.to_owned(),
            side: Side::Yes as i32,
            action: Action::Sell as i32,
            price_ticks: 50,
            count: 3,
            flags: 0,
            stp: 0,
        }))
        .await
        .expect("submit maker")
        .into_inner();
    assert!(maker.accepted, "maker was rejected: {}", maker.reject_code);

    let taker = client
        .submit_order(Request::new(MeSubmitOrderRequest {
            order_id: "smoke-taker".to_owned(),
            user_id: "taker".to_owned(),
            hold_id: "hold-taker".to_owned(),
            ticker: ticker.to_owned(),
            side: Side::Yes as i32,
            action: Action::Buy as i32,
            price_ticks: 50,
            count: 3,
            flags: 0,
            stp: 0,
        }))
        .await
        .expect("submit taker")
        .into_inner();
    assert!(taker.accepted, "taker was rejected: {}", taker.reject_code);
    assert_eq!(taker.fills.len(), 1);
    assert_eq!(taker.fills[0].count, 3);
    assert_eq!(taker.fills[0].price_ticks, 50);
    assert_eq!(taker.fills[0].maker_order_id, "smoke-maker");
    assert_eq!(taker.fills[0].taker_order_id, "smoke-taker");

    let snapshot = client
        .get_book_snapshot(Request::new(GetBookSnapshotRequest {
            ticker: ticker.to_owned(),
            depth: 25,
        }))
        .await
        .expect("snapshot")
        .into_inner();
    assert!(snapshot.bids.is_empty() && snapshot.asks.is_empty());

    let mut stream = client
        .stream_executions(Request::new(StreamExecutionsRequest { from_global_seq: 0 }))
        .await
        .expect("execution stream")
        .into_inner();
    let mut observed_fill = false;
    while let Ok(Some(Ok(event))) =
        tokio::time::timeout(std::time::Duration::from_millis(500), stream.next()).await
    {
        if event.ticker == ticker {
            if let Some(sarvex_contracts::sarvex::v1::execution_event::Event::Fill(fill)) =
                event.event
            {
                observed_fill = fill.fill_id.starts_with("fill_");
                break;
            }
        }
    }
    assert!(
        observed_fill,
        "fill was not replayed from the C++ execution stream"
    );
}
