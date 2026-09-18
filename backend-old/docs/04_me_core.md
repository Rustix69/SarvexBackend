# 04 — `me-core` (Matching Engine) Deep Dive

The matching engine is the highest-stakes piece. This document covers exactly how Liquibook is integrated, what we wrap, what the demo skips, and what production adds.

---

## 1. Liquibook in one paragraph

Liquibook (github.com/enewhuis/liquibook) is a header-only C++ template library that implements a limit order book with price-time priority and matching. It supports limit, market, IOC, FOK, AON, and (preliminary) stop orders. Prices are integers ("atomic currency units"). The user supplies an `Order` class and chooses a pointer type (`MyOrder*` or `std::shared_ptr<MyOrder>`). Liquibook never owns orders. It does **not** provide threading, persistence, replication, or a wire protocol — those are our responsibility. The benchmark in the repo claims 2.0–2.5M inserts/sec on a single thread, so we have orders of magnitude of headroom even at production targets.

---

## 2. What `me-core` adds around Liquibook

```
                       me-core process (single process; sharded by ticker set)
   ┌────────────────────────────────────────────────────────────────────┐
   │  gRPC server (grpc++)  ─── SubmitOrder/CancelOrder/AmendOrder      │
   │     │                                                              │
   │     │ enqueue                                                      │
   │     ▼                                                              │
   │  Inbound MPSC ring (moodycamel::ReaderWriterQueue per producer;    │
   │                     fan-in to a single drain by sequencer)         │
   │     │                                                              │
   │     ▼                                                              │
   │  ┌──────── Sequencer thread (pinned to dedicated core) ──────────┐ │
   │  │  for each command:                                            │ │
   │  │   1. Assign global_seq, contract_seq                          │ │
   │  │   2. [PRODUCTION] Append to journal segment + fdatasync       │ │
   │  │      [DEMO] (skipped)                                         │ │
   │  │   3. Apply to liquibook::book::DepthOrderBook<OrderPtr>       │ │
   │  │   4. Listener callbacks fire synchronously → outbound ring    │ │
   │  └───────────────────────────────────────────────────────────────┘ │
   │     │                                                              │
   │     ▼                                                              │
   │  Outbound ring → publisher thread                                  │
   │     ├── NATS publish:                                              │
   │     │     md.book.<ticker>       (BookDelta events)                │
   │     │     md.trade.<ticker>      (public trade tape)               │
   │     │     md.ticker.<ticker>     (debounced top-of-book)           │
   │     │     exec.events            (all internal execution events)   │
   │     │     exec.fills.<ticker>    (fill-only internal stream)       │
   │     └── gRPC stream responses to subscribed routers                │
   │                                                                    │
   │  [PRODUCTION] Snapshotter thread (every 30s):                      │
   │     - Atomically copy book state                                   │
   │     - Serialize (Cap'n Proto)                                      │
   │     - Local NVMe + S3 upload                                       │
   │                                                                    │
   │  [DEMO] Boot-time restore:                                         │
   │     - Read all OPEN/PARTIAL orders from orders.orders WHERE        │
   │       status IN ('OPEN', 'PARTIAL')                                │
   │     - Replay each in order_id (Snowflake = chronological) order    │
   │     - Resume accepting traffic                                     │
   └────────────────────────────────────────────────────────────────────┘
```

---

## 3. Process model

### Threads (same in demo and production)

1. **gRPC server I/O** (4 threads) — accept RPCs, decode, push command to inbound ring.
2. **Sequencer / Match thread** (1 thread, pinned). **All book mutation happens here.** Single-writer.
3. **NATS publisher thread** (1 thread) — drains outbound ring, publishes to NATS.
4. **[PRODUCTION ONLY] Journal sync thread** — handles `fdatasync` batching.
5. **[PRODUCTION ONLY] Snapshotter thread** — low priority.

### CPU pinning (production)

Sequencer thread pinned via `pthread_setaffinity_np()` to a core isolated by kernel `isolcpus` and `nohz_full` boot parameters. Demo doesn't bother with this.

### Memory

In production: pre-allocated arena (`std::pmr::monotonic_buffer_resource`) for orders, fills, events. No `malloc` on the matching thread after warmup. In demo: regular `new`/`delete`; no perf-tuning.

---

## 4. Core data structures

```cpp
// services/me-core/src/sarva_order.h
#pragma once
#include <book/types.h>
#include <string>

class SarvaOrder {
public:
  SarvaOrder(const std::string& order_id,
             const std::string& user_id,
             const std::string& hold_id,
             const std::string& ticker,
             liquibook::book::Quantity qty,
             liquibook::book::Price price,
             bool is_buy,
             bool is_limit,
             uint32_t flags)
    : order_id_(order_id), user_id_(user_id), hold_id_(hold_id),
      ticker_(ticker), qty_(qty), price_(price),
      is_buy_(is_buy), is_limit_(is_limit), flags_(flags) {}

  // Liquibook required interface:
  bool   is_limit()  const { return is_limit_; }
  bool   is_buy()    const { return is_buy_; }
  liquibook::book::Price    price() const { return price_; }
  liquibook::book::Quantity order_qty() const { return qty_; }

  // Liquibook AON & stop interface (we don't enable AON in demo):
  bool   all_or_none() const { return flags_ & 0x10; }
  bool   immediate_or_cancel() const { return flags_ & 0x01; }
  liquibook::book::Price stop_price() const { return 0; }

  // Sarvex-specific accessors
  const std::string& sarva_order_id() const { return order_id_; }
  const std::string& user_id() const { return user_id_; }
  const std::string& hold_id() const { return hold_id_; }
  const std::string& ticker() const { return ticker_; }
  uint32_t flags() const { return flags_; }

private:
  std::string order_id_, user_id_, hold_id_, ticker_;
  liquibook::book::Quantity qty_;
  liquibook::book::Price    price_;
  bool is_buy_, is_limit_;
  uint32_t flags_;  // bit0=IOC, bit1=FOK, bit2=POST_ONLY, bit3=REDUCE_ONLY, bit4=AON
};
```

```cpp
// services/me-core/src/shard_state.h
#pragma once
#include <book/depth_order_book.h>
#include <absl/container/flat_hash_map.h>
#include <memory>
#include "sarva_order.h"

using OrderPtr = SarvaOrder*;
using Book     = liquibook::book::DepthOrderBook<OrderPtr>;

class SarvaListener;  // implements OrderListener + TradeListener + DepthListener

struct ShardState {
  // Stable map from ticker -> book; one book per contract.
  // Books are heap-allocated; never moved; raw OrderPtr stays valid.
  absl::flat_hash_map<std::string, std::unique_ptr<Book>> books;
  std::unique_ptr<SarvaListener> listener;

  // Pool of SarvaOrder (production) / std::unordered_map<order_id, unique_ptr>
  // (demo). Demo path: keep alive in a map keyed by order_id so cancel can find it.
  absl::flat_hash_map<std::string, std::unique_ptr<SarvaOrder>> orders_by_id;

  // Sequencer state
  uint64_t global_seq = 0;
  absl::flat_hash_map<std::string, uint64_t> contract_seq;
};
```

---

## 5. Single-thread sequencer loop

```cpp
// services/me-core/src/sequencer.cpp
void Sequencer::run() {
  while (!shutdown_.load()) {
    Command cmd;
    if (!inbound_ring_.try_dequeue(cmd)) {
      std::this_thread::yield();  // or backoff; demo can use sleep
      continue;
    }
    process_command(cmd);
  }
}

void Sequencer::process_command(const Command& cmd) {
  state_.global_seq++;
  auto& cseq = state_.contract_seq[cmd.ticker];
  cseq++;

  // PRODUCTION ONLY:
  // journal_.append(cmd, state_.global_seq, cseq);  // sync write+fdatasync

  switch (cmd.type) {
    case CommandType::SUBMIT:  apply_submit(cmd); break;
    case CommandType::CANCEL:  apply_cancel(cmd); break;
    case CommandType::AMEND:   apply_amend(cmd); break;
    case CommandType::ADD_BOOK: apply_add_book(cmd); break;
    case CommandType::CLOSE_BOOK: apply_close_book(cmd); break;
  }
}

void Sequencer::apply_submit(const Command& cmd) {
  auto it = state_.books.find(cmd.ticker);
  if (it == state_.books.end()) {
    publish_reject(cmd, "BOOK_NOT_FOUND");
    return;
  }
  Book& book = *it->second;

  // Materialize the order
  auto order = std::make_unique<SarvaOrder>(
    cmd.order_id, cmd.user_id, cmd.hold_id, cmd.ticker,
    cmd.qty, cmd.price_ticks,
    cmd.action == Action::BUY,
    cmd.price_ticks > 0,
    cmd.flags
  );

  // POST_ONLY check: if would match, reject before adding
  if ((cmd.flags & 0x04) && would_cross_book(book, *order)) {
    publish_reject(cmd, "POST_ONLY_WOULD_MATCH");
    return;
  }

  OrderPtr op = order.get();
  state_.orders_by_id[cmd.order_id] = std::move(order);

  // STP enforcement happens inside the listener (it sees each proposed fill).
  // Liquibook calls back synchronously during book.add().
  // Listener output includes full immutable fill facts: ticker, global_seq,
  // contract_seq, both order IDs, both hold IDs, both side/action pairs, fees.
  current_taker_user_ = op->user_id();
  current_stp_policy_ = cmd.stp;

  book.add(op);

  // After book.add() returns, listener has emitted all events to outbound ring.
  // If the order is fully cancelled by STP / IOC / FOK, it's no longer on book.
  current_taker_user_.clear();
}
```

---

## 6. Listener (the bridge from Liquibook to our event stream)

```cpp
// services/me-core/src/sarva_listener.h
#pragma once
#include <book/order_listener.h>
#include <book/trade_listener.h>
#include <book/depth_listener.h>
#include "sarva_order.h"

class SarvaListener
  : public liquibook::book::OrderListener<OrderPtr>
  , public liquibook::book::TradeListener<Book>
  , public liquibook::book::DepthListener<Book>
{
public:
  SarvaListener(ShardState& s, OutboundRing& out)
    : state_(s), out_(out) {}

  // OrderListener
  void on_accept(const OrderPtr& o) override;
  void on_reject(const OrderPtr& o, const char* reason) override;
  void on_fill(const OrderPtr& o, const OrderPtr& matched,
               liquibook::book::Quantity qty,
               liquibook::book::Cost cost) override;
  void on_cancel(const OrderPtr& o) override;
  void on_cancel_reject(const OrderPtr& o, const char* reason) override;
  void on_replace(const OrderPtr& o,
                  const int64_t& size_delta,
                  liquibook::book::Price new_price) override;
  void on_replace_reject(const OrderPtr& o, const char* reason) override;

  // TradeListener
  void on_trade(const Book* book,
                liquibook::book::Quantity qty,
                liquibook::book::Cost cost) override;

  // DepthListener
  void on_depth_change(const Book* book,
                       const liquibook::book::DepthOrderBook<OrderPtr>::DepthTracker* tracker) override;

private:
  ShardState& state_;
  OutboundRing& out_;
};
```

Implementation translates each callback to a protobuf `ExecutionEvent` / `MarketDataEvent` and pushes onto the outbound ring.

---

## 7. Self-Trade Prevention (STP)

Liquibook does not enforce STP natively. We intercept proposed fills via `on_fill`:

- **`STP_TAKER_AT_CROSS`:** if `taker.user_id == maker.user_id`, cancel the taker before further matches. We accomplish this by tracking the current taker (set in `apply_submit`), and on first same-user fill: call `book.cancel(taker_order)` immediately and skip subsequent fills. Liquibook will continue to call `on_fill` for any already-fired matches; we suppress them in the listener.
- **`STP_MAKER`:** cancel the resting maker order, allow the taker to continue matching against the next price level. Same mechanism: `book.cancel(maker_order)` inside the listener.

This logic is encapsulated in `sarva_listener.cpp` with clear unit tests. The matrix to cover:
- taker matches single same-user maker fully
- taker matches same-user maker partially, then opposing-user maker
- multi-level walk with same-user maker at top
- `STP_TAKER_AT_CROSS` vs `STP_MAKER` produce different outcomes for the same inputs

---

## 8. gRPC server

```cpp
// services/me-core/src/grpc_server.cpp
class MatchingEngineImpl final : public sarvex::v1::MatchingEngine::Service {
public:
  grpc::Status SubmitOrder(grpc::ServerContext* ctx,
                            const MeSubmitOrderRequest* req,
                            MeSubmitOrderResponse* resp) override {
    Command cmd = command_from_proto(*req);

    // Each RPC creates a per-request promise that the listener resolves
    // when this order's outcome is known. The listener finds the promise
    // by order_id in a thread-safe map.
    auto promise = pending_acks_.create(req->order_id());

    if (!inbound_ring_.try_enqueue(cmd)) {
      pending_acks_.cancel(req->order_id());
      return grpc::Status(grpc::RESOURCE_EXHAUSTED, "queue full");
    }

    // Wait for ack (with deadline)
    auto future = promise.get_future();
    if (future.wait_for(std::chrono::milliseconds(100)) != std::future_status::ready) {
      // The command may still be queued or already applied. Caller must treat
      // this as UNKNOWN, not as a reject, and reconcile by order_id.
      return grpc::Status(grpc::DEADLINE_EXCEEDED, "match timeout");
    }
    AckResult ack = future.get();
    fill_response_from_ack(ack, resp);
    return grpc::Status::OK;
  }

  // CancelOrder, AmendOrder, and CloseBook follow the same enqueue + ack pattern.
  // CloseBook stops new submits for the ticker, expires all resting orders,
  // emits terminal cancel/expire events with reason BOOK_CLOSED, and returns
  // close_global_seq/close_contract_seq. Settlement must not run until all fills
  // and terminal order events up to close_global_seq are persisted, ledger-posted
  // where applicable, and applied to position/risk/order state.

  grpc::Status StreamExecutions(grpc::ServerContext* ctx,
                                 const StreamExecutionsRequest* req,
                                 grpc::ServerWriter<ExecutionEvent>* writer) override {
    // Subscribe a per-connection channel; sequencer's publisher writes
    // ExecutionEvents to all active subscribers. Demo supports live streaming
    // plus a small in-memory replay window. Production serves historical replay
    // from JetStream by from_global_seq.
    auto sub = execution_subscribers_.add(req->from_global_seq());
    while (!ctx->IsCancelled()) {
      ExecutionEvent ev;
      if (sub->channel.dequeue_with_timeout(ev, 100ms)) {
        if (!writer->Write(ev)) break;
      }
    }
    execution_subscribers_.remove(sub);
    return grpc::Status::OK;
  }

private:
  ShardState& state_;
  InboundRing& inbound_ring_;
  PendingAckRegistry pending_acks_;
  ExecutionSubscriberRegistry execution_subscribers_;
};
```

---

## 9. Timeout and retry semantics

A gRPC `DEADLINE_EXCEEDED` after the command was enqueued is **not** a rejection. The command may still apply on the sequencer. Callers must mark the order `PENDING` and reconcile by `order_id` via `GetOrder`/execution stream before releasing holds or returning a terminal state to the client.

`RESOURCE_EXHAUSTED` before enqueue means the command did not enter the sequencer; the caller may reject and release the hold. Duplicate `order_id` is idempotent: me-core returns the cached terminal ack if available, or `IDEMPOTENCY_EXPIRED` if the in-memory demo cache has aged out. Production derives duplicate handling from the journal.

---

## 10. Demo recovery: rebuild from `orders.orders`

On startup, `me-core` does:

1. Connect to refdata-svc via gRPC; load all OPEN/HALTED/RESOLVING contracts.
2. For each contract: `book.add_book_if_missing()`.
3. Query `orders.orders` for `status IN ('OPEN', 'PARTIAL')` ordered by `order_id` (which is Snowflake; chronological). This is an explicit demo-only ownership exception; production uses journal + snapshot.
4. For each row: synthesize a `RESTORE_RESTING` command carrying the remaining quantity and original price/time priority. Restore commands do **not** emit accepts, fills, ledger jobs, or market data.
5. Validate the rebuilt book is not crossed. If it is crossed, fail readiness and require operator repair rather than silently matching during restore.
6. Open gRPC port; serve traffic.

This rebuilds the book deterministically enough for the demo. Caveat: any **fills emitted by me-core but not persisted by order-router before a crash** are lost in demo. The order-router fill outbox and demo smoke checks reduce this risk; production eliminates it with journal+snapshot.

```go
// Pseudocode of what me-core's restore loop does (transliterated)
db.Query("SELECT order_id, user_id, hold_id, ticker, side, action,
           price_ticks, remaining_count, post_only, reduce_only, stp
          FROM orders.orders
          WHERE status IN ('OPEN', 'PARTIAL')
          ORDER BY order_id").
    forEach(row => sequencer.feedRestoreRestingCommand(row));
```

---

## 11. Production recovery: journal + snapshot

(Documented for clarity; not built in demo.)

**Cold start:**
1. Pull latest snapshot from S3 (or local NVMe).
2. Decode → populate books.
3. Tail NATS JetStream subject `me.journal.<shard>` from `snapshot.global_seq + 1` to end.
4. Apply each command to the book.
5. Begin accepting traffic.

**Failover:**
- Standby `me-core` runs in *replica mode*: tails journal continuously, applies to its books, never writes.
- Kubernetes leader-election lease + readiness gates.
- On primary loss, standby completes journal drain (catch up the last few ms), promotes, advertises in K8s Service. p99 RTO target: 5 seconds. RPO: 0 (journal is the source of truth and is durably stored in JetStream R3 before primary acks any client).

This isn't built in demo. The hooks (journal/snapshot writer interfaces) are stubbed but present, so adding them is a per-method implementation rather than a refactor.

```cpp
// services/me-core/src/journal.h
class Journal {
public:
  virtual void append(const Command& cmd, uint64_t global_seq, uint64_t contract_seq) = 0;
  virtual void sync() = 0;
  virtual ~Journal() = default;
};

class NoOpJournal : public Journal {  // used in demo
public:
  void append(const Command&, uint64_t, uint64_t) override {}
  void sync() override {}
};

class JetStreamJournal : public Journal { /* production */ };
```

The sequencer holds a `std::unique_ptr<Journal>`; demo wires `NoOpJournal`, production wires `JetStreamJournal`. Zero code changes elsewhere.

---

## 12. Order ID generation

64-bit Snowflake assigned by `order-router`, **not** me-core. Layout:
```
[1 bit unused][41 bits ms epoch since 2026-01-01][10 bits router-pod-id][12 bits seq]
```

Order IDs are passed into `SubmitOrder`. me-core treats them as opaque strings (we keep them string-typed in the proto for forward-compat with non-numeric IDs in v2).

**Idempotency:** on duplicate `order_id`, me-core's `orders_by_id` lookup fires; we respond with the cached ack instead of re-applying. A bounded LRU (1M entries) keeps memory in check; expired entries return `IDEMPOTENCY_EXPIRED` to surface the rare case clearly.

---

## 13. CMake / build

```cmake
# services/me-core/CMakeLists.txt
cmake_minimum_required(VERSION 3.20)
project(me-core CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Liquibook (header-only; we vendor it under third_party/)
add_subdirectory(third_party/liquibook EXCLUDE_FROM_ALL)
# Actually Liquibook's CMake builds a static lib `liquibook` from src/book/*

find_package(absl REQUIRED)
find_package(gRPC REQUIRED CONFIG)
find_package(Protobuf REQUIRED CONFIG)
find_package(nats REQUIRED)

# Generate proto sources
set(PROTO_SRCS ../../proto/sarvex/v1/match.proto
               ../../proto/sarvex/v1/common.proto
               ../../proto/sarvex/v1/marketdata.proto)
protobuf_generate(...)

add_executable(me-core
  src/main.cpp
  src/grpc_server.cpp
  src/sequencer.cpp
  src/sarva_listener.cpp
  src/nats_publisher.cpp
  src/restore.cpp
  ${PROTO_SRCS_OUT}
)
target_link_libraries(me-core PRIVATE
  liquibook
  absl::flat_hash_map
  gRPC::grpc++
  protobuf::libprotobuf
  cnats
  pqxx  # for restore
)
```

### Dockerfile

```dockerfile
# services/me-core/Dockerfile
FROM ubuntu:24.04 AS builder
RUN apt-get update && apt-get install -y \
    build-essential cmake git ninja-build pkg-config \
    libabsl-dev libgrpc++-dev libprotobuf-dev protobuf-compiler protobuf-compiler-grpc \
    libnats-dev libpqxx-dev
WORKDIR /src
COPY . .
RUN cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release && cmake --build build

FROM ubuntu:24.04
RUN apt-get update && apt-get install -y \
    libabsl20240722 libgrpc++1 libprotobuf32 libnats3 libpqxx-7.9 && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/build/me-core /usr/local/bin/
EXPOSE 50051
ENTRYPOINT ["/usr/local/bin/me-core"]
```

---

## 14. What we explicitly don't enable

| Feature | Reason |
|---|---|
| AON orders | Liquibook AON has documented edge cases. Disable in demo and Phase 2; revisit only with clear LP demand. |
| Stop orders | Liquibook marks stops as "preliminary." Demo and Phase 2 ship without; Phase 3 if needed. |
| Iceberg orders | Not supported by Liquibook; not on roadmap. |
| Multi-leg / spread orders | Out of scope through Phase 3. |
| Cross-margin | Phase 2 risk redesign; not relevant to ME. |

---

## 15. Test plan

### Unit tests (`services/me-core/tests/`)

- `book_basic_test.cpp` — limit order rests, market order matches, partial fills.
- `tif_test.cpp` — IOC, FOK behaviour matrix.
- `post_only_test.cpp` — would-cross detection.
- `stp_test.cpp` — TAKER_AT_CROSS and MAKER variants over 10+ scenarios.
- `restore_test.cpp` — feed a synthetic order set, snapshot positions, restart, verify identical state.
- `idempotency_test.cpp` — duplicate `order_id` returns same ack.
- `cancel_test.cpp` — cancel resting, cancel partially filled.

### Integration tests

`tests/integration/` runs me-core against an in-memory NATS and a mocked refdata. Drives via gRPC. Verifies:
- 1000 orders submitted → consistent book state.
- Order/fill/cancel events emitted in correct sequence.
- Crash + restart → state matches pre-crash.

### Load tests (production phase)

`tools/me-loadgen/` is a Go binary that floods me-core with synthetic orders. Targets:
- 10,000 orders/sec sustained
- p99 RPC latency ≤ 2 ms (demo isn't load-tested)

---

## 16. Phase 1 (demo) ME scope checklist

- [x] Wrap Liquibook with `SarvaOrder` + `DepthOrderBook`
- [x] Single-shard, multi-book in-process
- [x] gRPC `SubmitOrder`, `CancelOrder`, `AmendOrder`, `GetBookSnapshot`, `StreamExecutions`
- [x] NATS publish to `md.book.*`, `md.trade.*`, `exec.events`, `exec.fills.*`
- [x] Restore from Postgres on boot
- [x] STP (`TAKER_AT_CROSS`, `MAKER`)
- [x] TIF (`GTC`, `IOC`, `FOK`)
- [x] POST_ONLY
- [ ] Journal writer (production)
- [ ] Snapshotter (production)
- [ ] Hot-standby replica (production)
- [ ] CPU pinning + memory arena (production)
- [ ] AON, stops (not on roadmap)
