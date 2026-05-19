# 02 — Protobuf Definitions

**These are frozen on Day 1 of Week 1.** Once written, they don't change during the demo build. Any change requires explicit team review.

All protos live in `proto/sarvaex/v1/`. Compile via `./scripts/proto-gen.sh` which generates Go (`services/*/gen/pb/`) and C++ (`services/me-core/gen/`) bindings.

---

## 1. `common.proto`

Shared types used across all services.

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";

enum Side {
  SIDE_UNSPECIFIED = 0;
  YES = 1;       // binary
  NO = 2;        // binary
  LONG = 3;      // scalar
  SHORT = 4;     // scalar
}

enum Action {
  ACTION_UNSPECIFIED = 0;
  BUY = 1;
  SELL = 2;
}

enum TimeInForce {
  TIF_UNSPECIFIED = 0;
  GTC = 1;       // good till canceled
  IOC = 2;       // immediate or cancel
  FOK = 3;       // fill or kill
}

enum SelfTradePreventionType {
  STP_UNSPECIFIED = 0;
  STP_TAKER_AT_CROSS = 1;  // cancel taker if would match own resting
  STP_MAKER = 2;            // cancel resting maker
}

enum OrderStatus {
  ORDER_STATUS_UNSPECIFIED = 0;
  PENDING = 1;    // durable order row exists; ME outcome not final yet
  OPEN = 2;       // resting on book
  PARTIAL = 3;    // partially filled, remainder resting
  FILLED = 4;     // fully filled
  CANCELLED = 5;
  REJECTED = 6;
  EXPIRED = 7;
}

enum ContractKind {
  CONTRACT_KIND_UNSPECIFIED = 0;
  BINARY = 1;
  SCALAR = 2;
}

enum ContractState {
  CONTRACT_STATE_UNSPECIFIED = 0;
  DRAFT = 1;
  LISTED = 2;        // visible, not yet trading
  OPEN = 3;          // trading active
  CLOSED = 4;        // trading closed, awaiting resolution
  RESOLVING = 5;     // oracle/settlement in flight
  SETTLED = 6;       // payouts complete
  CANCELLED_CONTRACT = 7;
  HALTED = 8;         // admin/risk halt; may resume to OPEN or close/cancel
}

// All monetary values in protobuf are int64 micro_usdc (1 USDC = 1,000,000)
// All prices are int64 ticks (binary: cents 1-99; scalar: contract-defined)
// All quantities are int64 lots
```

---

## 2. `order.proto`

Used by `gw-rest` → `order-router` and Frontend ↔ `gw-rest`.

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";
import "sarvaex/v1/common.proto";

service OrderRouter {
  rpc SubmitOrder(SubmitOrderRequest) returns (SubmitOrderResponse);
  rpc CancelOrder(CancelOrderRequest) returns (CancelOrderResponse);
  rpc AmendOrder(AmendOrderRequest) returns (AmendOrderResponse);
  rpc GetOrder(GetOrderRequest) returns (Order);
  rpc ListOrders(ListOrdersRequest) returns (ListOrdersResponse);
  rpc ListFills(ListFillsRequest) returns (ListFillsResponse); // internal/recovery
}

message SubmitOrderRequest {
  string user_id = 1;
  string client_order_id = 2;   // idempotency key (per user)
  string ticker = 3;
  Side side = 4;
  Action action = 5;
  int64 price_ticks = 6;        // 0 = market order
  int64 count = 7;              // quantity in lots
  TimeInForce tif = 8;
  bool post_only = 9;
  bool reduce_only = 10;
  SelfTradePreventionType stp = 11;
  google.protobuf.Timestamp expires_at = 12;  // optional GTD
  string idempotency_key = 13;  // request-level, separate from client_order_id
}

message SubmitOrderResponse {
  Order order = 1;
  repeated Fill fills = 2;       // immediate fills if any
  string reject_code = 3;        // empty if accepted
  string reject_reason = 4;
}

message CancelOrderRequest {
  string user_id = 1;
  string order_id = 2;           // exchange order id
  string client_order_id = 3;    // alternative lookup
}

message CancelOrderResponse {
  Order order = 1;
  string reject_code = 2;
  string reject_reason = 3;
}

message AmendOrderRequest {
  string user_id = 1;
  string order_id = 2;
  int64 new_price_ticks = 3;     // 0 = unchanged
  int64 new_count = 4;            // 0 = unchanged
}

message AmendOrderResponse {
  Order order = 1;
  string reject_code = 2;
}

message GetOrderRequest {
  string user_id = 1;
  oneof key {
    string order_id = 2;
    string client_order_id = 3;
  }
}

message ListOrdersRequest {
  string user_id = 1;
  string ticker = 2;        // optional filter
  OrderStatus status = 3;   // optional filter; 0 = all
  int32 limit = 4;
  string cursor = 5;
}

message ListOrdersResponse {
  repeated Order orders = 1;
  string next_cursor = 2;
}

message ListFillsRequest {
  string ticker = 1;              // optional; empty = all tickers
  uint64 from_global_seq = 2;     // inclusive; 0 = beginning
  uint64 to_global_seq = 3;       // inclusive; 0 = no upper bound
  int32 limit = 4;
  string cursor = 5;
}

message ListFillsResponse {
  repeated FillRecord fills = 1;
  string next_cursor = 2;
}

message Order {
  string order_id = 1;
  string client_order_id = 2;
  string user_id = 3;
  string ticker = 4;
  Side side = 5;
  Action action = 6;
  int64 price_ticks = 7;
  int64 count = 8;
  int64 filled_count = 9;
  int64 remaining_count = 10;
  TimeInForce tif = 11;
  bool post_only = 12;
  bool reduce_only = 13;
  SelfTradePreventionType stp = 14;
  OrderStatus status = 15;
  google.protobuf.Timestamp created_at = 16;
  google.protobuf.Timestamp updated_at = 17;
  google.protobuf.Timestamp expires_at = 18;
  string hold_id = 19;           // from ledger
  int64 avg_fill_price_ticks = 20;
}

message Fill {
  string fill_id = 1;
  string order_id = 2;
  string ticker = 3;
  int64 price_ticks = 4;
  int64 count = 5;
  Side aggressor_side = 6;
  int64 fee_micro_usdc = 7;
  google.protobuf.Timestamp ts = 8;
  uint64 seq = 9;                 // per-ticker monotonic
}

message FillRecord {
  string fill_id = 1;
  string ticker = 2;
  uint64 global_seq = 3;
  uint64 contract_seq = 4;
  string maker_order_id = 5;
  string taker_order_id = 6;
  string maker_user_id = 7;
  string taker_user_id = 8;
  string maker_hold_id = 9;
  string taker_hold_id = 10;
  Side maker_side = 11;
  Action maker_action = 12;
  Side taker_side = 13;
  Action taker_action = 14;
  int64 price_ticks = 15;
  int64 count = 16;
  Side aggressor_side = 17;
  int64 maker_fee_micro_usdc = 18;
  int64 taker_fee_micro_usdc = 19;
  google.protobuf.Timestamp ts = 20;
}
```

---

## 3. `match.proto`

Internal interface: `order-router` ↔ `me-core`. Stays the same in demo and production.

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";
import "google/protobuf/empty.proto";
import "sarvaex/v1/common.proto";

service MatchingEngine {
  // Lifecycle
  rpc AddBook(AddBookRequest) returns (google.protobuf.Empty);
  rpc CloseBook(CloseBookRequest) returns (CloseBookResponse);

  // Order entry (synchronous)
  rpc SubmitOrder(MeSubmitOrderRequest) returns (MeSubmitOrderResponse);
  rpc CancelOrder(MeCancelOrderRequest) returns (MeCancelOrderResponse);
  rpc AmendOrder(MeAmendOrderRequest) returns (MeAmendOrderResponse);

  // Read
  rpc GetBookSnapshot(GetBookSnapshotRequest) returns (BookSnapshot);

  // Streaming exec events (router subscribes for fills/cancels)
  rpc StreamExecutions(StreamExecutionsRequest) returns (stream ExecutionEvent);
}

message AddBookRequest {
  string ticker = 1;
  ContractKind kind = 2;
  int64 tick_size = 3;
  int64 min_price_ticks = 4;
  int64 max_price_ticks = 5;
}

message CloseBookRequest {
  string ticker = 1;
}

message CloseBookResponse {
  string ticker = 1;
  uint64 close_global_seq = 2;     // high-water mark settlement must catch up to
  uint64 close_contract_seq = 3;
}

message MeSubmitOrderRequest {
  string order_id = 1;           // assigned by router (Snowflake)
  string user_id = 2;
  string hold_id = 3;             // from ledger
  string ticker = 4;
  Side side = 5;
  Action action = 6;
  int64 price_ticks = 7;
  int64 count = 8;
  uint32 flags = 9;               // bit0=IOC, bit1=FOK, bit2=POST_ONLY, bit3=REDUCE_ONLY
  SelfTradePreventionType stp = 10;
}

// NATS subjects carrying ExecutionEvent:
//   exec.events                 - all execution events, ordered by global_seq in a single demo ME
//   exec.fills.<ticker>         - fill-only stream for internal consumers
//   exec.user.<user_id>         - sanitized per-user order events published by order-router
//   exec.fills.user.<user_id>   - sanitized per-user fills published by order-router
//
// In production, sharded ME instances publish exec.events.<shard> and
// exec.fills.<ticker>. Consumers persist offsets and must detect sequence gaps.
message MeSubmitOrderResponse {
  string order_id = 1;
  bool accepted = 2;
  string reject_code = 3;
  repeated MeFill fills = 4;       // immediate matches
  uint64 contract_seq = 5;          // for ordering
  uint64 global_seq = 6;
}

message MeCancelOrderRequest {
  string order_id = 1;
}

message MeCancelOrderResponse {
  string order_id = 1;
  bool cancelled = 2;
  string reject_code = 3;
  int64 cancelled_qty = 4;          // remaining qty that was on book
}

message MeAmendOrderRequest {
  string order_id = 1;
  int64 new_price_ticks = 2;
  int64 new_count = 3;
}

message MeAmendOrderResponse {
  string order_id = 1;
  bool amended = 2;
  string reject_code = 3;
}

message GetBookSnapshotRequest {
  string ticker = 1;
  int32 depth = 2;                  // levels per side
}

message BookSnapshot {
  string ticker = 1;
  uint64 seq = 2;
  google.protobuf.Timestamp ts = 3;
  repeated PriceLevel bids = 4;
  repeated PriceLevel asks = 5;
}

message PriceLevel {
  int64 price_ticks = 1;
  int64 total_qty = 2;
  int32 order_count = 3;
}

message StreamExecutionsRequest {
  uint64 from_global_seq = 1;       // resume point
}

message ExecutionEvent {
  uint64 global_seq = 1;
  uint64 contract_seq = 2;
  string ticker = 3;
  google.protobuf.Timestamp ts = 4;
  oneof event {
    OrderAcceptedEvent accepted = 5;
    OrderRejectedEvent rejected = 6;
    MeFill fill = 7;
    OrderCancelledEvent cancelled = 8;
    OrderAmendedEvent amended = 9;
    BookDelta book_delta = 10;
  }
}

message OrderAcceptedEvent {
  string order_id = 1;
}
message OrderRejectedEvent {
  string order_id = 1;
  string reject_code = 2;
}
message OrderCancelledEvent {
  string order_id = 1;
  int64 cancelled_qty = 2;
  string reason_code = 3;           // USER_CANCEL | BOOK_CLOSED | ADMIN_CANCEL | STP
}
message OrderAmendedEvent {
  string order_id = 1;
  int64 new_price_ticks = 2;
  int64 new_count = 3;
}

message MeFill {
  string fill_id = 1;
  string maker_order_id = 2;
  string taker_order_id = 3;
  string maker_user_id = 4;
  string taker_user_id = 5;
  int64 price_ticks = 6;
  int64 count = 7;
  Side aggressor_side = 8;
  string ticker = 9;
  uint64 global_seq = 10;
  uint64 contract_seq = 11;
  google.protobuf.Timestamp ts = 12;
  string maker_hold_id = 13;
  string taker_hold_id = 14;
  Side maker_side = 15;
  Action maker_action = 16;
  Side taker_side = 17;
  Action taker_action = 18;
  int64 maker_fee_micro_usdc = 19;
  int64 taker_fee_micro_usdc = 20;
}

message BookDelta {
  Side side = 1;
  int64 price_ticks = 2;
  int64 qty_delta = 3;              // signed
  int64 new_total_qty = 4;
}
```

---

## 4. `ledger.proto`

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";
import "google/protobuf/empty.proto";
import "google/protobuf/struct.proto";

service Ledger {
  rpc PostTransaction(PostTransactionRequest) returns (PostTransactionResponse);
  rpc PlaceHold(PlaceHoldRequest) returns (PlaceHoldResponse);
  rpc ReleaseHold(ReleaseHoldRequest) returns (google.protobuf.Empty);
  rpc CommitHold(CommitHoldRequest) returns (google.protobuf.Empty);
  rpc GetBalance(GetBalanceRequest) returns (Balance);
  rpc GetAccountHistory(GetAccountHistoryRequest) returns (GetAccountHistoryResponse);

  // Admin (demo only — replaced by chain integration in prod)
  rpc AdminCreditDeposit(AdminCreditDepositRequest) returns (google.protobuf.Empty);
}

message PostTransactionRequest {
  string idempotency_key = 1;
  string reason_code = 2;       // FILL | DEPOSIT | WITHDRAWAL | SETTLEMENT | FEE | etc
  repeated LedgerEntry entries = 3;  // must sum to zero per currency
  google.protobuf.Struct metadata = 4;
}

message PostTransactionResponse {
  string tx_id = 1;
  google.protobuf.Timestamp posted_at = 2;
}

message LedgerEntry {
  string account_code = 1;       // e.g. "LIAB:USER:u_42:CASH"
  string direction = 2;          // "DR" or "CR"
  int64 amount_micro_usdc = 3;   // positive
  string memo = 4;
}

message PlaceHoldRequest {
  string idempotency_key = 1;    // typically order_id
  string user_id = 2;
  int64 amount_micro_usdc = 3;
  string reason = 4;             // "ORDER:RBI-JUN26-CUT25"
}

message PlaceHoldResponse {
  string hold_id = 1;
}

message ReleaseHoldRequest {
  string idempotency_key = 1;
  string hold_id = 2;
  int64 amount_micro_usdc = 3;   // partial release; 0 = remaining uncommitted
  string reason_code = 4;
}

message CommitHoldRequest {
  string idempotency_key = 1;
  string hold_id = 2;
  int64 commit_amount_micro_usdc = 3;  // moves from HOLDS to destination_account_code
  int64 release_amount_micro_usdc = 4; // refund to CASH; 0 = none
  string destination_account_code = 5; // e.g. LIAB:HOUSE:UNSETTLED_TRADES:<ticker>
  string reason_code = 6;
  repeated LedgerEntry additional_entries = 7;  // e.g. fees
}

message GetBalanceRequest {
  string user_id = 1;
}

message Balance {
  string user_id = 1;
  int64 cash_micro_usdc = 2;
  int64 held_micro_usdc = 3;
  int64 total_micro_usdc = 4;
}

message GetAccountHistoryRequest {
  string user_id = 1;
  int32 limit = 2;
  string cursor = 3;
}

message GetAccountHistoryResponse {
  repeated LedgerEntryRecord entries = 1;
  string next_cursor = 2;
}

message LedgerEntryRecord {
  string tx_id = 1;
  string account_code = 2;
  string direction = 3;
  int64 amount_micro_usdc = 4;
  int64 running_balance_micro_usdc = 5;
  string reason_code = 6;
  google.protobuf.Timestamp posted_at = 7;
  string memo = 8;
}

message AdminCreditDepositRequest {
  string user_id = 1;
  int64 amount_micro_usdc = 2;
  string note = 3;
}
```

---

## 5. `risk.proto`

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/empty.proto";
import "sarvaex/v1/common.proto";

service Risk {
  rpc PreTradeCheck(PreTradeCheckRequest) returns (PreTradeCheckResponse);
  rpc GetUserLimits(GetUserLimitsRequest) returns (UserLimits);
  rpc UpdateUserLimits(UpdateUserLimitsRequest) returns (google.protobuf.Empty);
}

message PreTradeCheckRequest {
  string user_id = 1;
  string ticker = 2;
  Side side = 3;
  Action action = 4;
  int64 price_ticks = 5;
  int64 count = 6;
}

message PreTradeCheckResponse {
  bool approved = 1;
  string reject_code = 2;
  string reject_reason = 3;
  int64 required_hold_micro_usdc = 4;
  int64 projected_position = 5;    // current + working + this order
}

message GetUserLimitsRequest {
  string user_id = 1;
}

message UserLimits {
  string user_id = 1;
  int32 kyc_tier = 2;
  int64 max_order_size_micro_usdc = 3;
  int64 daily_loss_limit_micro_usdc = 4;
  map<string, int64> per_contract_position_limit = 5;  // ticker -> max qty
}

message UpdateUserLimitsRequest {
  UserLimits limits = 1;
}
```

---

## 6. `refdata.proto`

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";
import "google/protobuf/empty.proto";
import "google/protobuf/struct.proto";
import "sarvaex/v1/common.proto";

service RefData {
  rpc GetContract(GetContractRequest) returns (Contract);
  rpc ListContracts(ListContractsRequest) returns (ListContractsResponse);
  rpc TransitionState(TransitionStateRequest) returns (google.protobuf.Empty);

  // Admin
  rpc UpsertContract(UpsertContractRequest) returns (Contract);
  rpc GetEvent(GetEventRequest) returns (Event);
}

message Contract {
  string ticker = 1;
  string event_ticker = 2;
  string series_ticker = 3;
  ContractKind kind = 4;
  string question = 5;             // for binaries
  string underlying = 6;             // for scalars (description)
  int64 tick_size = 7;
  int64 min_price_ticks = 8;
  int64 max_price_ticks = 9;
  int64 lower_bound_ticks = 10;     // scalars only
  int64 upper_bound_ticks = 11;
  int64 multiplier_micro_usdc = 12; // scalars only
  int64 max_order_size = 13;
  int64 position_limit_per_user = 14;
  ContractState state = 15;
  google.protobuf.Timestamp listed_at = 16;
  google.protobuf.Timestamp open_at = 17;
  google.protobuf.Timestamp close_at = 18;
  google.protobuf.Timestamp expected_resolution_at = 19;
  string settlement_source = 20;     // URL or descriptor
  string oracle_policy = 21;         // SINGLE_SOURCE | MULTI_SOURCE_ATTEST | ADMIN
  google.protobuf.Struct settlement_rule = 22; // contract-specific mapping from event outcome to payout
  uint64 close_global_seq = 23;       // set when trading closes through me-core
}

message Event {
  string event_ticker = 1;
  string series_ticker = 2;
  string title = 3;
  string description = 4;
  google.protobuf.Timestamp expected_resolution_at = 5;
}

message GetContractRequest {
  string ticker = 1;
}

message ListContractsRequest {
  ContractState state = 1;       // 0 = all
  string series_ticker = 2;
  int32 limit = 3;
  string cursor = 4;
}

message ListContractsResponse {
  repeated Contract contracts = 1;
  string next_cursor = 2;
}

message TransitionStateRequest {
  string ticker = 1;
  ContractState new_state = 2;
  string reason = 3;
}

message UpsertContractRequest {
  Contract contract = 1;
}

message GetEventRequest {
  string event_ticker = 1;
}
```

---

## 7. `oracle.proto`

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";

service Oracle {
  rpc ProposeResolution(ProposeResolutionRequest) returns (Resolution);
  rpc FinalizeResolution(FinalizeResolutionRequest) returns (Resolution);
  rpc GetResolution(GetResolutionRequest) returns (Resolution);

  // Admin (demo: trigger from JSON file or admin console)
  rpc AdminForceResolution(AdminForceResolutionRequest) returns (Resolution);
}

message ProposeResolutionRequest {
  string event_ticker = 1;
  int64 numeric_value = 2;         // for scalar events (e.g., CPI in bps)
  string categorical_value = 3;    // e.g., "CUT_25" or "HOLD"
  string source = 4;
  string attestor_id = 5;
  bytes signature = 6;
}

message FinalizeResolutionRequest {
  string event_ticker = 1;
}

message GetResolutionRequest {
  string event_ticker = 1;
}

message Resolution {
  string event_ticker = 1;
  int64 numeric_value = 2;
  string categorical_value = 3;
  ResolutionStatus status = 4;
  repeated Attestation attestations = 5;
  google.protobuf.Timestamp proposed_at = 6;
  google.protobuf.Timestamp finalized_at = 7;
}

message Attestation {
  string attestor_id = 1;
  string source = 2;
  int64 numeric_value = 3;
  string categorical_value = 4;
  bytes signature = 5;
  google.protobuf.Timestamp observed_at = 6;
}

enum ResolutionStatus {
  RESOLUTION_STATUS_UNSPECIFIED = 0;
  RESOLUTION_PENDING = 1;
  RESOLUTION_PROPOSED = 2;
  RESOLUTION_FINALIZED = 3;
  RESOLUTION_DISPUTED = 4;
}

message AdminForceResolutionRequest {
  string event_ticker = 1;
  int64 numeric_value = 2;
  string categorical_value = 3;
  string admin_user_id = 4;
  string justification = 5;
}
```

---

## 8. `settlement.proto`

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";

service Settlement {
  rpc SettleContract(SettleContractRequest) returns (SettlementResult);
  rpc GetSettlement(GetSettlementRequest) returns (SettlementResult);
}

message SettleContractRequest {
  string ticker = 1;
  string event_ticker = 2;
}

message GetSettlementRequest {
  string ticker = 1;
}

message SettlementResult {
  string ticker = 1;
  google.protobuf.Timestamp settled_at = 2;
  int64 winner_payout_per_contract_micro_usdc = 3;
  int64 total_payout_micro_usdc = 4;
  int32 positions_settled = 5;
}
```

---

## 9. `marketdata.proto`

Used over NATS subjects. Same proto for demo and production.

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";
import "sarvaex/v1/common.proto";

// Subjects:
//   md.book.<ticker>          - OrderBookDeltaEvent
//   md.trade.<ticker>         - TradeEvent
//   md.lifecycle.<ticker>     - ContractLifecycleEvent
//   md.ticker.<ticker>        - TickerEvent

message OrderBookDeltaEvent {
  string ticker = 1;
  uint64 seq = 2;
  google.protobuf.Timestamp ts = 3;
  repeated DeltaLevel deltas = 4;
}

message DeltaLevel {
  Side side = 1;
  int64 price_ticks = 2;
  int64 qty_delta = 3;       // signed
  int64 new_total_qty = 4;
}

message TradeEvent {
  string ticker = 1;
  string trade_id = 2;
  uint64 seq = 3;
  int64 price_ticks = 4;
  int64 count = 5;
  Side aggressor_side = 6;
  google.protobuf.Timestamp ts = 7;
}

message TickerEvent {
  string ticker = 1;
  uint64 seq = 2;
  int64 best_bid_ticks = 3;
  int64 best_bid_qty = 4;
  int64 best_ask_ticks = 5;
  int64 best_ask_qty = 6;
  int64 last_trade_ticks = 7;
  google.protobuf.Timestamp ts = 8;
}

message ContractLifecycleEvent {
  string ticker = 1;
  ContractState old_state = 2;
  ContractState new_state = 3;
  google.protobuf.Timestamp ts = 4;
  string reason = 5;
}
```

---

## 10. `audit.proto`

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";
import "google/protobuf/struct.proto";

// Subject: audit.events
message AuditEvent {
  uint64 event_seq = 1;            // monotonic across all services
  string service = 2;
  string type = 3;                  // e.g., "ORDER_SUBMITTED", "WITHDRAWAL_APPROVED"
  string actor = 4;                 // user_id, "service:<name>", "admin:<email>"
  string subject = 5;               // ticker, order_id, user_id, etc.
  google.protobuf.Struct payload = 6;
  bytes prev_hash = 7;              // production: tamper-evident chain
  bytes hash = 8;
  google.protobuf.Timestamp ts = 9;
  string trace_id = 10;
}
```

---

## 11. `position.proto`

```protobuf
syntax = "proto3";
package sarvaex.v1;
option go_package = "github.com/sarvaex/proto/gen/go/sarvaex/v1;sarvaexv1";

import "google/protobuf/timestamp.proto";

service Position {
  rpc GetPosition(GetPositionRequest) returns (UserPosition);
  rpc ListPositions(ListPositionsRequest) returns (ListPositionsResponse);
  rpc ListPositionsByContract(ListPositionsByContractRequest) returns (ListPositionsResponse);
  rpc GetOpenInterest(GetOpenInterestRequest) returns (OpenInterest);
}

message UserPosition {
  string user_id = 1;
  string ticker = 2;
  int64 net_qty = 3;                  // signed
  int64 avg_cost_micro_usdc = 4;
  int64 realized_pnl_micro_usdc = 5;
  int64 unrealized_pnl_micro_usdc = 6;
  google.protobuf.Timestamp updated_at = 7;
  uint64 last_global_seq = 8;          // latest fill seq incorporated for this user/contract
}

message GetPositionRequest {
  string user_id = 1;
  string ticker = 2;
}

message ListPositionsRequest {
  string user_id = 1;
  bool include_closed = 2;
}

message ListPositionsResponse {
  repeated UserPosition positions = 1;
  string next_cursor = 2;
}

message ListPositionsByContractRequest {
  string ticker = 1;
  bool include_closed = 2;
  int32 limit = 3;
  string cursor = 4;
  uint64 min_global_seq = 5;           // settlement waits until service has applied through close_global_seq
}

message GetOpenInterestRequest {
  string ticker = 1;
}

message OpenInterest {
  string ticker = 1;
  int64 total_open_long = 2;
  int64 total_open_short = 3;
}
```

---

## 12. REST API (`gw-rest`)

OpenAPI sketch. Same in demo and production (production adds auth headers, rate limits).

```yaml
openapi: 3.0.3
info:
  title: SarvaEX REST API
  version: v1

paths:
  /v1/orders:
    post:
      operationId: submitOrder
      security: [{bearerAuth: []}]
      parameters:
        - {name: Idempotency-Key, in: header, required: true, schema: {type: string, maxLength: 64}}
      requestBody:
        required: true
        content:
          application/json:
            schema: {$ref: '#/components/schemas/SubmitOrderInput'}
      responses:
        '201': {description: Accepted, content: {application/json: {schema: {$ref: '#/components/schemas/OrderOutput'}}}}
        '202': {description: ME outcome pending after enqueue timeout; poll order by id}
        '402': {description: Insufficient funds}
        '422': {description: Validation or risk reject}
        '429': {description: Rate limited}
        '503': {description: Engine unavailable}

    get:
      operationId: listOrders
      parameters:
        - {name: ticker, in: query, schema: {type: string}}
        - {name: status, in: query, schema: {type: string, enum: [pending, open, partial, filled, cancelled, rejected, expired]}}
        - {name: limit, in: query, schema: {type: integer, default: 50, maximum: 500}}
        - {name: cursor, in: query, schema: {type: string}}
      responses:
        '200': {description: OK}

  /v1/orders/{order_id}:
    delete:
      operationId: cancelOrder
      parameters: [{name: order_id, in: path, required: true, schema: {type: string}}]
      responses:
        '200': {description: Cancelled}
        '404': {description: Not found}

  /v1/orders/{order_id}/amend:
    post:
      operationId: amendOrder
      parameters: [{name: order_id, in: path, required: true, schema: {type: string}}]
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties:
                new_price_ticks: {type: integer}
                new_count: {type: integer}
      responses:
        '200': {description: OK}

  /v1/markets:
    get:
      operationId: listMarkets
      parameters:
        - {name: state, in: query, schema: {type: string}}
      responses:
        '200': {description: OK}

  /v1/markets/{ticker}:
    get:
      operationId: getMarket
      responses:
        '200': {description: OK}

  /v1/markets/{ticker}/orderbook:
    get:
      operationId: getOrderbook
      parameters:
        - {name: depth, in: query, schema: {type: integer, default: 20, maximum: 100}}
      responses:
        '200': {description: OK}

  /v1/positions:
    get:
      operationId: listPositions
      responses: {'200': {description: OK}}

  /v1/account/balance:
    get:
      operationId: getBalance
      responses: {'200': {description: OK}}

  /v1/account/history:
    get:
      operationId: getAccountHistory
      responses: {'200': {description: OK}}

  # Admin endpoints (Phase 2 will require separate auth)
  /v1/admin/deposits/credit:
    post:
      operationId: adminCreditDeposit
      requestBody:
        content:
          application/json:
            schema:
              type: object
              required: [user_id, amount_micro_usdc]
              properties:
                user_id: {type: string}
                amount_micro_usdc: {type: integer, format: int64}
                note: {type: string}
      responses: {'200': {description: OK}}

components:
  schemas:
    SubmitOrderInput:
      type: object
      required: [ticker, side, action, count]
      properties:
        client_order_id: {type: string, maxLength: 64}
        ticker: {type: string}
        side: {type: string, enum: [yes, no, long, short]}
        action: {type: string, enum: [buy, sell]}
        count: {type: integer, minimum: 1}
        price_ticks: {type: integer, minimum: 0}    # 0 = market
        time_in_force: {type: string, enum: [gtc, ioc, fok], default: gtc}
        post_only: {type: boolean, default: false}
        reduce_only: {type: boolean, default: false}
        self_trade_prevention_type: {type: string, enum: [taker_at_cross, maker]}
        expires_at: {type: string, format: date-time}

    OrderOutput:
      type: object
      properties:
        order_id: {type: string}
        client_order_id: {type: string}
        ticker: {type: string}
        side: {type: string}
        action: {type: string}
        status: {type: string}
        price_ticks: {type: integer}
        count: {type: integer}
        filled_count: {type: integer}
        remaining_count: {type: integer}
        fills: {type: array, items: {$ref: '#/components/schemas/FillOutput'}}
        created_at: {type: string, format: date-time}

    FillOutput:
      type: object
      properties:
        fill_id: {type: string}
        price_ticks: {type: integer}
        count: {type: integer}
        fee_micro_usdc: {type: integer}
        ts: {type: string, format: date-time}

  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
      bearerFormat: JWT
```

---

## 13. WSS Protocol (`gw-ws`)

JSON over WebSocket. Same protocol in demo and production.

**Connect:** `wss://api.sarvaex.com/v1/stream` (production) / `ws://localhost:8081/v1/stream` (demo).

**Subscribe:**
```json
{ "id": 1, "cmd": "subscribe",
  "params": { "channels": ["orderbook_delta", "trades", "ticker"],
              "tickers": ["RBI-JUN26-CUT25"] } }
```

**Channels:**
- `orderbook_snapshot` — full snapshot on first subscription
- `orderbook_delta` — incremental updates
- `trades` — tape
- `ticker` — top-of-book + last trade
- `user_orders` — private; requires auth
- `user_fills` — private
- `user_balance` — private
- `market_lifecycle` — contract state transitions

**Server message envelope:**
```json
{ "type": "orderbook_delta",
  "sid": 42,
  "seq": 12891,
  "msg": { "ticker": "...", "deltas": [...] } }
```

**Snapshot/delta rule:** server subscribes and buffers `md.book.<ticker>` deltas before fetching the snapshot, sends the snapshot, then replays buffered deltas with `seq > snapshot.seq`. This prevents the snapshot race where a delta is published between the gRPC snapshot and the NATS subscription.

**Gap recovery:** if client detects `seq` gap, sends:
```json
{ "id": 2, "cmd": "resync", "params": { "sid": 42 } }
```
Server repeats the buffer -> snapshot -> replay flow. Production also offers a 1-second rolling delta cache.

**Heartbeats:** server sends WS Ping every 15s. Client must respond with Pong. After 2 missed Pongs, server closes connection.

**Auth:** authenticated channels require a `{ "cmd": "auth", "params": { "jwt": "..." } }` message immediately after connect.
