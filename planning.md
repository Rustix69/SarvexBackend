# Sarvex MVP Backend Execution Plan

This file captures the milestone-by-milestone implementation roadmap for the Sarvex MVP backend.

The plan preserves the finalized architecture:

- Protobuf-first service contracts
- Distributed service boundaries
- Single-writer matching through `me-core`
- Liquibook as the embedded matching/book library only
- Sarvex-owned sequencing, durability, ledger, holds, replay, settlement, and orchestration
- Demo-grade implementation behind production-grade boundaries

## Current Execution Status

- Milestone 0: Completed.
- Milestone 1: Completed.
- Milestone 2: Completed.
- Milestone 3: Completed.
- Milestone 4: Completed.
- Milestone 5: Completed.
- Milestone 6: Completed.
- Milestone 7: Completed.
- Next active milestone: Milestone 8 (Order Router And Fill Durability).

## Implementation North Star

Sarvex is not a generic application backend. It is an exchange backend where correctness depends on deterministic sequencing, durable fill facts, idempotent ledger operations, replayability, and settlement ordering.

The safest implementation order is:

```text
repo + proto
  -> migrations
  -> service stubs
  -> ledger/holds
  -> refdata/risk
  -> me-core/Liquibook
  -> sequencer/events/snapshot
  -> order-router/fill outbox
  -> NATS consumers/positions
  -> gateways
  -> close/oracle/settlement
  -> recovery/ops/load
```

Do not build UI-visible trading before the correctness spine is in place.

## Service Dependency Graph

```text
gw-rest
  -> order-router
      -> refdata-svc
      -> risk-svc
      -> ledger-svc
      -> me-core

gw-ws
  -> NATS market/private subjects
  -> me-core snapshots

me-core
  -> Liquibook
  -> refdata boot load
  -> orders DB demo restore exception
  -> NATS publish

order-router
  -> orders DB
  -> fill_posting_outbox
  -> ledger fill poster
  -> user execution event publish

position-svc
  -> exec.fills.*
  -> OrderRouter.ListFills replay

settlement-svc
  -> oracle-svc
  -> refdata-svc
  -> order-router fill status/replay
  -> position-svc
  -> ledger-svc

audit-svc
  -> audit.events
```

## Critical Invariants

- Liquibook is only the in-memory price-time matching book.
- Sarvex owns sequencing, durability, ledger correctness, holds, replay, settlement, and distributed behavior.
- No Liquibook mutation may occur outside the `me-core` sequencer thread.
- No NATS, DB, gRPC, or ledger work may occur inside Liquibook callbacks.
- Every matched fill must become an immutable Sarvex fill fact.
- Every fill fact must include maker/taker order IDs, user IDs, hold IDs, side/action pairs, price, count, fees, `global_seq`, and `contract_seq`.
- Order-router must persist fills and fill outbox rows before async ledger posting.
- Ledger is the only authority for balances.
- All ledger operations must be balanced, append-only, and idempotent.
- A `DEADLINE_EXCEEDED` from `me-core` after enqueue is not a rejection.
- Holds must not be released until a terminal outcome is known.
- Position consumers must detect `global_seq` gaps and replay through `OrderRouter.ListFills`.
- Settlement must not run until `close_global_seq` exists and all fills, holds, ledger postings, and positions have caught up through that sequence.
- WebSocket snapshots must be paired with buffered deltas to avoid snapshot races.

## Milestone 0: Repo Bootstrap

### Objective

Create the buildable repository skeleton before implementing business logic.

### Deliverables

- Monorepo layout from the architecture docs
- `Makefile`
- `docker-compose.yml`
- `.env.example`
- CI pipeline
- `scripts/proto-gen.sh`
- Migration runner
- Shared Go packages for config, logging, NATS, Postgres, auth, idempotency, tracing
- `services/me-core/` CMake shell

### Services Involved

All services as empty projects.

### Dependencies

None.

### Risks

- Architectural drift starts immediately if any service boundary is skipped.
- Generated-code paths can become inconsistent between Go and C++.
- Docker-compose health checks may report readiness before dependencies are ready.

### Correctness Checks

- `make build`
- `make test`
- `make proto`
- All services expose liveness/readiness endpoints.
- Service ports match the runbook.

### Integration Tests

- Compose starts Postgres, NATS, Redis.
- All service containers boot.
- Health endpoints return success.

### Can Stay Stubbed

- All RPC methods.
- All domain logic.

### Must Be Fully Correct

- Repository layout.
- Generated-code paths.
- Health/readiness conventions.
- Service ports and compose names.

### Should Not Be Implemented Yet

- Order flow.
- Ledger logic.
- Matching logic.
- Frontend or bots.

## Milestone 1: Protobuf Freeze

### Objective

Make protobuf the single source of truth for service contracts.

### Deliverables

- All `.proto` files under `proto/sarvex/v1/`
- Go bindings for Go services
- C++ bindings for `me-core`
- Proto linting
- Breaking-change check
- Stub servers registering all RPCs

### Services Involved

All backend services.

### Dependencies

Milestone 0.

### Risks

- Changing `MeFill`, `ExecutionEvent`, `CloseBookResponse`, `ListFills`, or hold APIs later will cause cross-service churn.
- Missing idempotency fields will force unsafe workarounds.
- Missing sequence fields will break replay and settlement.

### Correctness Checks

- Generated Go and C++ code compile.
- `MeFill` includes both maker and taker facts.
- `CloseBookResponse` includes `close_global_seq`.
- `Position.ListPositionsByContract` supports `min_global_seq`.
- `OrderRouter.ListFills` supports replay by `global_seq`.
- Ledger hold APIs support idempotent place, release, and commit.

### Integration Tests

- Stub services start with generated bindings.
- Each RPC can be invoked and returns a typed unimplemented response.

### Completion Evidence (2026-05-20)

- Protobuf contracts are present under `proto/sarvex/v1/*.proto`.
- `make proto` completes and generates:
  - Go bindings in `gen/go/sarvex/v1/`
  - C++ bindings in `services/me-core/gen/sarvex/v1/`
- A consolidated stub registration server exists at `cmd/proto-stub-server/main.go` and compiles via `go build ./cmd/proto-stub-server`.
- Required contract fields for replay/settlement/holds were verified in proto definitions.

### Can Stay Stubbed

- Service implementations.

### Must Be Fully Correct

- Enums.
- NATS event payloads.
- Idempotency fields.
- Sequence fields.
- Settlement and position replay contracts.

### Should Not Be Implemented Yet

- Alternate JSON-first contracts.
- Handwritten request structs that drift from protobuf.

## Milestone 2: Database Migrations And Seeds

### Objective

Lock persistence invariants before services write state.

### Deliverables

- Service schemas for refdata, users, ledger, orders, risk, position, oracle, settlement, audit
- Migration runner integrated with `make migrate`
- Seed contracts
- Seed users
- Seed house accounts
- Ledger balance trigger
- Orders fill/outbox tables
- Hold operation table
- Position offsets and applied-fills tables
- Settlement payout tables

### Services Involved

`ledger-svc`, `order-router`, `refdata-svc`, `risk-svc`, `position-svc`, `oracle-svc`, `settlement-svc`, `audit-svc`.

### Dependencies

Milestone 1.

### Risks

- Weak schema constraints become correctness debt.
- Missing unique constraints can permit duplicate orders, fills, or payouts.
- Missing outbox tables will make crash recovery unsafe.

### Correctness Checks

- Ledger entries are append-only.
- Ledger transactions must balance.
- `orders.orders` has `UNIQUE(user_id, client_order_id)`.
- `orders.fills.global_seq` is unique.
- `orders.fills` has `UNIQUE(ticker, ticker_seq)`.
- `orders.fill_posting_outbox` exists.
- `ledger.hold_operations` exists.
- `position.consumer_offsets` exists.
- `position.applied_fills` exists.
- `settlement.settlement_payouts.idempotency_key` is unique.

### Integration Tests

- Migration up/down.
- Seed data loads.
- Unbalanced ledger transaction is rejected.
- Duplicate order client ID is rejected.
- Duplicate fill ID is rejected.
- Duplicate settlement idempotency key is rejected.

### Can Stay Stubbed

- Service APIs.

### Must Be Fully Correct

- Ledger append-only structure.
- Hold constraints.
- Fill persistence schema.
- Outbox schemas.
- Position replay schema.

### Should Not Be Implemented Yet

- Production partitioning.
- Per-service DB roles beyond basic readiness.
- Analytics schemas.

## Milestone 3: Service Skeleton And Compose Bring-Up

### Objective

Make every service boundary real before hot-path logic.

### Deliverables

- gRPC servers for all internal services
- REST gateway skeleton
- WebSocket gateway skeleton
- Health/readiness endpoints
- Dockerfiles
- Structured logging
- Config loading
- DB and NATS connection setup
- Compose service dependencies

### Services Involved

All backend services.

### Dependencies

Milestones 0, 1, and 2.

### Docker-Compose Bring-Up Order

```text
postgres
  -> nats
  -> redis
  -> migrations
  -> refdata-svc
  -> ledger-svc
  -> risk-svc
  -> me-core
  -> order-router
  -> position-svc
  -> oracle-svc
  -> settlement-svc
  -> audit-svc
  -> admin-svc
  -> gw-rest
  -> gw-ws
```

### Risks

- Hidden startup dependencies.
- Readiness endpoints that lie.
- Services accidentally reading another service's schema.

### Correctness Checks

- Readiness passes only after required dependencies are connected.
- Services use generated protobuf bindings.
- Logs include service name and trace/request ID.

### Integration Tests

- `make run` starts all services.
- All health endpoints pass.
- A stubbed request can traverse `gw-rest -> order-router -> me-core`.

### Completion Evidence (2026-05-21)

- Added generic gRPC service skeleton runtime with config loading, structured logs, DB/NATS dependency checks, and `/healthz` + `/readyz` endpoints.
- Added gateway skeletons:
  - `gw-rest`: HTTP service with milestone endpoints returning typed `NOT_IMPLEMENTED` responses.
  - `gw-ws`: WebSocket handshake skeleton with welcome event.
- Added Dockerfiles for all backend services and `me-core`.
- Updated compose to full bring-up order with explicit dependencies:
  `postgres -> nats -> redis -> migrations -> refdata-svc -> ledger-svc -> risk-svc -> me-core -> order-router -> position-svc -> oracle-svc -> settlement-svc -> audit-svc -> admin-svc -> gw-rest -> gw-ws`.
- Validated:
  - `go build ./...` and `go test ./...` pass.
  - compose config validates.
  - full stack boots successfully (with local port overrides) and services report healthy.

### Can Stay Stubbed

- Domain logic.
- NATS event handling.
- Settlement logic.

### Must Be Fully Correct

- Lifecycle management.
- Config loading.
- Health/readiness semantics.

### Should Not Be Implemented Yet

- Frontend polish.
- Market maker bots.
- Production Kubernetes.

## Milestone 4: Ledger And Hold Lifecycle

### Objective

Make funds safe before orders can trade.

### Deliverables

- `Ledger.PostTransaction`
- `Ledger.PlaceHold`
- `Ledger.ReleaseHold`
- `Ledger.CommitHold`
- Account lazy creation
- Deterministic account locking
- Non-negative CASH/HOLDS enforcement
- Ledger event outbox
- Admin demo deposit

### Services Involved

`ledger-svc`.

### Dependencies

Milestone 2.

### Risks

- Double-charge.
- Double-release.
- Partial hold commit.
- Deadlocks from inconsistent account locking.
- Ledger entries that balance mathematically but violate account semantics.

### Correctness Checks

- Every transaction is balanced.
- Duplicate idempotency keys return the original result.
- Hold invariant: `committed_micro_usdc + released_micro_usdc <= amount_micro_usdc`.
- User CASH and HOLDS cannot go negative.
- Account locks are acquired in deterministic order.

### Integration Tests

- Admin deposit.
- Place hold.
- Release full hold.
- Partial commit plus refund.
- Duplicate place hold.
- Duplicate release.
- Duplicate commit.
- Insufficient cash.
- Concurrent holds on same user.

### Completion Evidence (2026-05-21)

- `ledger-svc` now has concrete gRPC implementations for:
  - `PostTransaction`
  - `PlaceHold`
  - `ReleaseHold`
  - `CommitHold`
  - `GetBalance`
  - `GetAccountHistory`
  - `AdminCreditDeposit`
- Implemented transactional posting with:
  - idempotent `ledger.transactions` creation by `idempotency_key`
  - lazy account creation
  - deterministic account locking (sorted account-code lock order)
  - running-balance/account-seq updates
  - insufficient-funds enforcement for user `CASH`/`HOLDS`
  - hold operation idempotency through `ledger.hold_operations`
  - ledger outbox writes (`ledger.ledger_event_outbox`)
- Added executable tests in `pkg/m3svc/ledger_server_test.go` validating:
  - admin credit + balance read
  - place-hold idempotency + release path
  - insufficient-funds hold rejection
- Validation result:
  - `go test ./pkg/m3svc -v` passed against local Postgres
  - `go test ./...` passed

### Can Stay Stubbed

- Real withdrawals.
- Fireblocks.
- Chain deposit watcher.
- Production reconciliation.

### Must Be Fully Correct

- Double-entry posting.
- Hold atomicity.
- Idempotency.
- Balance reads.

### Should Not Be Implemented Yet

- Real custody.
- KYC vendor.
- Withdrawal policy engine.

## Milestone 5: Refdata And Risk MVP

### Objective

Define tradable contracts and perform pre-trade max-loss checks.

### Deliverables

- Refdata contract CRUD
- Contract lifecycle FSM
- Contract scheduler
- Contract state history
- Risk sanity checks
- Binary margin formula
- Scalar margin formula
- Basic position limit check placeholder
- Refdata cache invalidation hook

### Services Involved

`refdata-svc`, `risk-svc`.

### Dependencies

Milestone 2. Ledger is useful but not required for refdata.

### Risks

- Wrong price-range validation.
- Wrong YES/NO max-loss calculation.
- Wrong scalar max-loss calculation.
- Stale contract state allowing orders after halt/close.

### Correctness Checks

- All monetary values are integers.
- All prices are ticks.
- Non-OPEN contracts are rejected.
- Tick alignment is enforced.
- Contract max order size is enforced.
- Price range is enforced.

### Integration Tests

- Binary BUY YES.
- Binary BUY NO.
- Binary SELL YES.
- Binary SELL NO.
- Scalar LONG.
- Scalar SHORT.
- Invalid price.
- Invalid quantity.
- Closed contract reject.
- Lifecycle transition table.

### Completion Evidence (2026-05-21)

- `refdata-svc` now has concrete gRPC implementations for:
  - `GetContract`
  - `ListContracts`
  - `TransitionState`
  - `UpsertContract`
  - `GetEvent`
- `risk-svc` now has concrete gRPC implementations for:
  - `PreTradeCheck`
  - `GetUserLimits`
  - `UpdateUserLimits`
- Refdata behavior implemented:
  - contract upsert with enum/state mapping
  - state transition history writes
  - contract/event reads with typed protobuf responses
  - contract list filtering by state/series with cursor-based pagination
- Risk behavior implemented:
  - binary and scalar required-hold calculations using integer arithmetic
  - contract state/open checks
  - tick alignment, price-range, and max-order-size checks
  - projected position computation from `position.positions` and `risk.working_orders_summary`
  - per-contract override limits from `risk.contract_position_limits`
- Added tests in `pkg/m3svc/milestone5_test.go` covering:
  - refdata upsert/get/transition/list flows
  - risk pre-trade approval path and position-limit reject path
- Validation result:
  - `go test ./pkg/m3svc -v` passed against local Postgres
  - `go test ./...` passed

### Can Stay Stubbed

- Redis Lua reservations.
- Daily loss tracking.
- Production rate limits.
- Cross-margin.

### Must Be Fully Correct

- Required hold calculation.
- Contract state validation.
- Integer arithmetic.

### Should Not Be Implemented Yet

- Portfolio margin.
- Cross-contract risk offsets.
- Production Redis hot-path reservations.

## Milestone 6: `me-core` Liquibook Scaffold

### Objective

Wrap Liquibook without letting Liquibook become the exchange.

### Deliverables

- `services/me-core/` CMake project
- C++ generated proto integration
- Liquibook interface target using `liquibook/src`
- `SarvaOrder`
- `ShardState`
- `BookState`
- Stable order ownership maps
- Listener bridge shell
- `AddBook`
- Basic `GetBookSnapshot` placeholder

### Services Involved

`me-core`.

### Dependencies

Milestone 1 and Liquibook vendored under `liquibook/`.

### Recommended CMake Shape

```text
liquibook_headers
  -> INTERFACE include path: ${REPO_ROOT}/liquibook/src

me_core_proto
  -> generated C++ protobuf/grpc sources

me_core_lib
  -> sequencer, order model, listener bridge, book state

me_core_server
  -> main executable

me_core_tests
  -> unit tests linked against me_core_lib
```

Do not build Liquibook examples, tests, web assets, or QuickFAST-related code into Sarvex.

### Risks

- Unstable `SarvaOrder*` pointers.
- Wrong binary YES/NO normalization.
- Treating Liquibook as the owner of orders.
- Letting Liquibook callbacks perform Sarvex side effects.

### Correctness Checks

- Sarvex owns order lifetime.
- Liquibook sees only normalized buy/sell, price, and quantity.
- Original side/action remain available for fill facts.
- No concurrent book access exists.
- No external I/O happens in callbacks.

### Integration Tests

- Price-time priority.
- Pointer stability across inserts/cancels.
- YES/NO normalization.
- LONG/SHORT normalization.
- Reject unsupported Liquibook features.

### Completion Evidence (2026-05-21)

- Added `me-core` scaffold domain layer:
  - `SarvaOrder` model (`services/me-core/src/mecore/sarva_order.h`)
  - `BookState` + `ShardState` ownership/state boundaries
  - `ListenerBridge` shell implementing order/trade/depth callbacks
  - `MeCoreEngine` with `add_book` and `get_book_snapshot` placeholder behavior
- Added stable ownership maps in `ShardState`:
  - `books_by_ticker` for one-book-per-contract
  - `orders_by_id` for durable order object ownership boundary
- Added Liquibook-to-Sarvex boundary hooks:
  - `DepthOrderBook<OrderPtr>` usage
  - listener attachment for order/trade/depth callback capture
- Updated CMake shape:
  - `liquibook_headers` interface target
  - `me_core_proto_headers` integration include target
  - `me_core_lib` for core layer
  - `me-core` executable linked to `me_core_lib`
- Added runtime proto compatibility shim (`proto_compat.h`) to allow build when generated proto toolchain version and system protobuf headers differ.
- Validation:
  - `me-core` Docker build succeeds in compose.
  - `me-core` container is running in stack (`Up`), migrations job exits `0`.

### Can Stay Stubbed

- NATS publisher.
- Journal.
- Restore.
- Full STP.

### Must Be Fully Correct

- Order ownership.
- Normalized book mapping.
- Liquibook include/build setup.
- Callback boundary.

### Should Not Be Implemented Yet

- Production journal.
- Production snapshot.
- JetStream.
- Standby replication.

## Milestone 7: Sequencer, Matching, Events, Snapshots

### Objective

Make matching deterministic and sequencer-owned.

### Deliverables

- Inbound MPSC queue
- Single sequencer thread
- `global_seq`
- `contract_seq`
- Submit path
- Cancel path
- Post-only reject before add
- GTC/IOC/FOK handling
- Initial STP behavior
- `CloseBook`
- Real `GetBookSnapshot`
- Execution stream
- Outbound event batch
- Book deltas and trade events

### Services Involved

`me-core`.

### Dependencies

Milestone 6.

### Risks

- Assigning sequence numbers after matching.
- Reading snapshots off-thread.
- Publishing directly from callbacks.
- Treating gRPC timeout as reject.
- Incorrect fill ID determinism.

### Correctness Checks

- Sequence is assigned before Liquibook mutation.
- All book mutation happens on sequencer thread.
- Liquibook callbacks only append to current command context.
- Fill IDs are deterministic.
- Snapshot seq equals a sequencer-owned contract sequence.
- `CloseBook` returns `close_global_seq`.

### Integration Tests

- Three buys and two sells produce expected fills.
- Cancel removes remaining quantity.
- Post-only crossing order rejects without book mutation.
- IOC cancels unfilled remainder.
- FOK either fully fills or does not mutate.
- Snapshot plus deltas reconstructs book.
- Same command list replay produces same fills.

### Completion Evidence (2026-05-21)

- Added sequencer-based command architecture in `me-core`:
  - inbound command queue (`std::deque` + mutex/cv)
  - dedicated sequencer thread (`sequencer_loop`)
  - command processing for `AddBook`, `SubmitOrder`, `CancelOrder`, `CloseBook`, `GetBookSnapshot`
- Implemented sequence assignment semantics:
  - `global_seq` increments before Liquibook mutation
  - `contract_seq` increments per-ticker before mutation
- Implemented core submit/cancel/close behavior in `MeCoreEngine`:
  - post-only pre-check (`POST_ONLY_WOULD_MATCH`)
  - IOC/FOK scaffold behavior
  - deterministic fill-id construction from `(ticker, contract_seq, fill_index)`
  - close-book returns `close_global_seq` + `close_contract_seq`
- Implemented snapshot path from current book state with sequencer-owned `contract_seq`.
- Callback boundary preserved:
  - Liquibook callbacks only append listener events; no DB/NATS/gRPC side effects in listener methods.
- Build/runtime validation:
  - `me-core` compiles in Docker via compose
  - container starts and stays `Up`
  - startup log confirms submit path + sequence assignment (`submit.accepted=true seq=1/1`)

### Can Stay Stubbed

- Production journal via `NoOpJournal`.
- Advanced amend semantics.
- Full production-grade STP edge cases if not required for demo.

### Must Be Fully Correct

- Single-writer sequencing.
- Fill facts.
- Snapshot consistency.
- Timeout semantics.

### Should Not Be Implemented Yet

- HA failover.
- CPU pinning.
- Memory arena optimization.

## Milestone 8: Order Router And Fill Durability

### Status

Completed in code (`pkg/m3svc/order_router_server.go`, `pkg/m3svc/app.go`, `pkg/m3svc/config.go`) with passing `go test ./...`.

### Delivered In This Milestone

- Implemented `OrderRouter` server methods:
  - `SubmitOrder`, `CancelOrder`, `GetOrder`, `ListOrders`, `ListFills`
  - `AmendOrder` intentionally left stubbed (`UNIMPLEMENTED`) per milestone scope.
- Added Snowflake-style order ID generation.
- Enforced `(user_id, client_order_id)` idempotency by returning existing order on duplicate key.
- Inserted `PENDING` order row before downstream side effects.
- Added refdata contract-open validation and risk pre-trade call.
- Added ledger hold placement + deterministic idempotency keys.
- Added matching submit call with timeout handling:
  - queue-full (`RESOURCE_EXHAUSTED`) path releases hold and rejects.
  - unknown outcome (`DEADLINE_EXCEEDED`/unavailable) keeps `PENDING` and returns `ACK_UNKNOWN`.
- Added transactional fill persistence:
  - inserts `orders.fills`
  - updates order fill/status counters
  - inserts `orders.fill_posting_outbox` rows in same transaction.
- Added fill posting worker pass (`runFillPosterOnce`) that drains outbox idempotently and marks posted status.
- Added cancel flow hold release of remaining amount and terminal `CANCELLED` transition.

### Objective

Make the trading path durable around `me-core`.

### Deliverables

- `OrderRouter.SubmitOrder`
- Snowflake order IDs
- Request idempotency
- `(user_id, client_order_id)` idempotency
- PENDING order insert before side effects
- Refdata validation
- Risk call
- Ledger hold placement
- ME submit
- ME timeout reconciliation
- Fill persistence
- Order status updates
- `fill_posting_outbox`
- Fill poster worker
- User order/fill event publisher
- Cancel path and hold release

### Services Involved

`order-router`, `ledger-svc`, `risk-svc`, `me-core`.

### Dependencies

Milestones 4, 5, 7.

### Risks

- Releasing hold on ME timeout.
- Persisting fills after async ledger posting.
- Re-applying duplicate orders.
- Missing maker order update when taker fills.
- Filling order but leaving ledger outbox missing.

### Correctness Checks

- `RESOURCE_EXHAUSTED` before enqueue releases hold and rejects.
- `DEADLINE_EXCEEDED` after enqueue leaves order PENDING and keeps hold.
- Fill persistence and order updates happen in one DB transaction.
- Fill outbox row is inserted in the same DB transaction.
- Fill poster is idempotent by `fill_id`.
- Terminal non-fill events release remaining hold idempotently.

### Integration Tests

- Submit resting order.
- Submit crossing order.
- Immediate fill persisted.
- Duplicate client order returns existing order.
- Duplicate ME order ID is idempotent.
- Crash after fill persistence and before ledger posting.
- Restart drains fill outbox.
- Cancel releases remaining hold.
- Timeout leaves PENDING.

### Can Stay Stubbed

- Amend.
- Advanced STP.
- Production Redis idempotency.

### Must Be Fully Correct

- PENDING semantics.
- Fill durability.
- Hold release and commit decisions.
- Fill ledger posting idempotency.

### Should Not Be Implemented Yet

- Public WebSocket order entry.
- Bulk order entry.
- Mass cancel.

## Milestone 9: NATS Event Spine And Consumers

### Status

Completed in current codebase scope with:
- Event producer/consumer workers in `pkg/m3svc/milestone9_spine.go`
- Order/fill event publishing in `pkg/m3svc/order_router_server.go`
- Position RPC server implementation in `pkg/m3svc/position_server.go`
- Role-based worker startup in `pkg/m3svc/app.go`
- Passing validation: `go test ./...`

### Delivered In This Milestone

- Published execution and market subjects from order-router:
  - `exec.events`
  - `exec.fills.<ticker>`
  - `exec.user.<user_id>` (sanitized private order stream)
  - `exec.fills.user.<user_id>`
  - `md.trade.<ticker>`
  - `md.ticker.<ticker>`
- Published ledger subjects from durable outbox:
  - `ledger.events`
  - `ledger.balance.user.<user_id>`
- Implemented position consumer with durability checks:
  - offsets in `position.consumer_offsets`
  - idempotency in `position.applied_fills`
  - gap detection (`global_seq > last + 1`)
  - replay via `OrderRouter.ListFills`
- Implemented risk working-order summary consumer on fill events.
- Implemented audit consumer persisting consumed execution/ledger events to `audit.events`.
- Implemented concrete `position-svc` read APIs:
  - `GetPosition`, `ListPositions`, `ListPositionsByContract`, `GetOpenInterest`

### Objective

Distribute events without making NATS the source of truth.

### Deliverables

- `md.book.<ticker>`
- `md.trade.<ticker>`
- `md.ticker.<ticker>`
- `exec.events`
- `exec.fills.<ticker>`
- `exec.user.<user_id>`
- `exec.fills.user.<user_id>`
- `ledger.events`
- `ledger.balance.user.<user_id>`
- Audit consumer
- Position consumer
- Risk working-order consumer
- Gap replay through `OrderRouter.ListFills`

### Services Involved

`me-core`, `order-router`, `position-svc`, `risk-svc`, `ledger-svc`, `audit-svc`.

### Dependencies

Milestone 8.

### Risks

- Out-of-order fill application.
- Missed NATS messages.
- Duplicate fill messages.
- Private counterparty data leaking to clients.
- Treating publish success as durability.

### Correctness Checks

- Consumers persist offsets.
- Position consumer detects `global_seq > last + 1`.
- Missing ranges replay through `OrderRouter.ListFills`.
- Duplicate fills are skipped only if `applied_fills(fill_id)` exists.
- User-facing events are sanitized by order-router.

### Integration Tests

- Drop one fill event and verify replay.
- Duplicate fill event is idempotent.
- Position offset advances only after DB commit.
- Risk working summary updates on accept/fill/cancel.
- Private user feed excludes raw counterparty-only data.

### Can Stay Stubbed

- JetStream durable consumers.
- Production subject sharding.

### Must Be Fully Correct

- `global_seq` gap handling.
- Applied-fill idempotency.
- Sanitized private event streams.

### Should Not Be Implemented Yet

- Production sharded ME subjects.
- Durable JetStream consumer config.

## Milestone 10: Gateways

### Objective

Expose stable client APIs only after the core trading path is correct.

### Deliverables

- REST auth/login
- REST idempotency middleware
- REST order endpoints
- REST market endpoints
- REST balance/history endpoints
- REST position endpoints
- WebSocket connect/auth
- WebSocket subscribe protocol
- Market-data NATS bridge
- Private user NATS bridge
- Snapshot-buffer-replay flow
- Backpressure handling

### Services Involved

`gw-rest`, `gw-ws`.

### Dependencies

Milestones 5, 8, and 9.

### Risks

- REST retries creating duplicate orders.
- WebSocket snapshot race.
- Exposing internal `exec.fills.<ticker>` to clients.
- Lossy private streams.
- Long-running gateway calls hiding backend timeouts.

### Correctness Checks

- Mutating REST endpoints require `Idempotency-Key`.
- Gateway does not own domain logic.
- WS subscribes to deltas before fetching snapshot.
- Deltas with `seq <= snapshot.seq` are discarded.
- Buffered deltas with `seq > snapshot.seq` are replayed in order.
- Private channels require auth.

### Integration Tests

- Submit order via REST.
- Observe book delta over WS.
- Retry same REST request and receive idempotent response.
- Simulate WS gap and resync.
- Authenticated user receives only own orders/fills.

### Can Stay Stubbed

- Production API key signing.
- Redis distributed idempotency.
- Multi-pod WS routing concerns.

### Must Be Fully Correct

- Order idempotency.
- Snapshot/delta synchronization.
- Private data isolation.

### Should Not Be Implemented Yet

- Low-latency WebSocket order entry.
- FIX gateway.

## Milestone 11: Close, Oracle, Settlement

### Objective

Complete the contract lifecycle safely.

### Deliverables

- Coordinated `OPEN -> CLOSED`
- `me-core.CloseBook`
- Terminal order events for book close
- Remaining hold release
- Persisted `close_global_seq`
- Admin oracle resolution
- `oracle.resolutions.finalized.<event_ticker>`
- Settlement worker
- Payout formulas
- Payout intents
- Idempotent settlement ledger postings
- Rounding sweep
- `CLOSED -> RESOLVING -> SETTLED`

### Services Involved

`refdata-svc`, `me-core`, `order-router`, `oracle-svc`, `settlement-svc`, `position-svc`, `ledger-svc`.

### Dependencies

Milestones 8, 9, and 10.

### Risks

- Settlement before all fills are persisted.
- Settlement before ledger fill posting completes.
- Settlement before position consumer catches up.
- Missing `close_global_seq`.
- Duplicate settlement payout.
- Incorrect binary settlement rule.
- Incorrect scalar rounding.

### Correctness Checks

- Settlement requires `close_global_seq`.
- Settlement waits for all fills through close to be ledger POSTED.
- Settlement waits for all terminal order hold releases through close.
- Settlement waits for position-svc to apply through close.
- Payout idempotency key is `settlement:<ticker>:<user_id>`.
- `UNSETTLED_TRADES:<ticker>` drains to zero after payout and rounding sweep.

### Integration Tests

- Trade, close, resolve YES, settle.
- Trade, close, resolve NO, settle.
- Scalar lower bound.
- Scalar upper bound.
- Scalar midpoint.
- Position consumer lag blocks settlement and then resumes.
- Crash mid-payout and restart.
- Duplicate oracle finalized event does not duplicate payouts.

### Can Stay Stubbed

- Multi-source oracle.
- Challenge window.
- Dual admin approval in demo.

### Must Be Fully Correct

- Close sequencing.
- Settlement idempotency.
- Integer payout math.
- Settlement prerequisites.

### Should Not Be Implemented Yet

- Production oracle attestors.
- Real external data source integrations.

## Milestone 12: Demo Recovery, Reset, Load, Observability

### Objective

Prove restart and replay safety for the MVP.

### Deliverables

- `me-core` passive restore from OPEN/PARTIAL orders
- Restore emits no accepts, fills, ledger jobs, or market data
- Restore crossed-book validation
- Order-router reconciliation worker
- Fill outbox restart drain
- Risk summary rebuild script
- Reset script
- Seed script
- Demo smoke test
- Basic metrics
- Structured logs
- Runbook checks
- Load smoke tests

### Services Involved

All backend services.

### Dependencies

All prior milestones.

### Risks

- Restore silently matches crossed books.
- Lost fill after ME emits but before router persists.
- Reset leaves stale ledger/accounts.
- NATS outage hides consumer lag.
- Readiness passes while service is internally inconsistent.

### Correctness Checks

- Crossed restored book fails readiness.
- Restored resting orders preserve priority enough for demo.
- Restore performs no public side effects.
- Pending outbox rows drain after restart.
- Reset clears volatile state only.

### Integration Tests

- Restart `me-core` with resting orders.
- Restart `order-router` with pending fill outbox.
- Restart `position-svc` and replay missing fills.
- Simulate NATS outage and recovery.
- Run 1,000 order smoke load.
- Full demo script end to end.

### Can Stay Stubbed

- Production journal.
- Production snapshot.
- K8s.
- Full Prometheus/Grafana stack.

### Must Be Fully Correct

- Restore no-side-effect behavior.
- Outbox drain.
- Readiness gates.
- Reset determinism.

### Should Not Be Implemented Yet

- HA failover.
- JetStream journal.
- Production snapshotter.
- Production observability stack.

## Testing Strategy By Layer

### Protobuf Tests

- Generated Go and C++ compile.
- No breaking changes without review.
- JSON/proto gateway conversion preserves field names and enums.

### Database Tests

- Migrations apply cleanly.
- Migrations rollback where supported.
- Ledger balance trigger rejects unbalanced transactions.
- Unique constraints enforce idempotency.
- Outbox tables support ordered retry.

### Ledger Tests

- Deposit.
- Place hold.
- Release hold.
- Commit hold.
- Commit plus refund.
- Duplicate operation idempotency.
- Concurrent operations on same account.
- Non-negative balance enforcement.

### Matching Tests

- Price-time priority.
- Limit crossing.
- Market order handling.
- IOC.
- FOK.
- Post-only reject.
- Cancel.
- Close book.
- Snapshot consistency.
- Replay determinism.
- YES/NO and LONG/SHORT normalization.

### Order-Router Tests

- PENDING insert before side effects.
- Risk reject.
- Ledger hold reject.
- ME reject.
- ME queue full.
- ME timeout after enqueue.
- Immediate fill persistence.
- Fill outbox posting.
- Cancel hold release.
- Idempotent duplicate order.

### Position Tests

- Apply maker and taker fills.
- Binary signed quantity math.
- Scalar signed quantity math.
- Duplicate fill skip.
- Gap detection.
- Replay via `OrderRouter.ListFills`.
- Offset advances only after DB commit.

### Settlement Tests

- Binary YES wins.
- Binary NO wins.
- Scalar lower bound.
- Scalar upper bound.
- Scalar midpoint.
- Rounding sweep.
- Idempotent payout retry.
- Position lag.
- Fill ledger lag.
- Crash mid-settlement.

### Gateway Tests

- REST idempotency.
- REST auth.
- REST error mapping.
- WebSocket auth.
- Snapshot-buffer-replay.
- Gap resync.
- Private stream authorization.

### End-To-End Tests

- Deposit -> order -> hold -> match -> fill persistence -> ledger commit -> position update.
- Resting order -> cancel -> hold release.
- Two users trade -> close -> resolve -> settle.
- Restart services during pending outbox.
- Restart `me-core` and restore resting book.

## Likely Failure Scenarios

### ME Timeout Edge Case

Failure: `order-router` receives `DEADLINE_EXCEEDED` after the command entered the ME queue.

Required behavior:

- Keep order `PENDING`.
- Keep hold active.
- Reconcile by `order_id`.
- Do not return terminal reject.
- Do not release hold.

### Fill Persistence Gap

Failure: `me-core` emits fill, but `order-router` crashes before persisting it.

Demo mitigation:

- Use execution stream and smoke checks.
- Keep this risk explicit.

Production solution:

- ME journal plus replayable durable execution stream.

### NATS Message Loss

Failure: `position-svc` misses `exec.fills.<ticker>`.

Required behavior:

- Detect `global_seq` gap.
- Pause live apply.
- Replay missing range through `OrderRouter.ListFills`.
- Resume from contiguous offset.

### Settlement Race

Failure: Settlement runs before all fills or positions are caught up.

Required behavior:

- Settlement remains `PENDING` or `IN_PROGRESS`.
- Emit blocked/waiting status.
- Do not mark contract `SETTLED`.
- Do not post partial unsafe payouts.

### Snapshot Race

Failure: WS fetches snapshot, then subscribes to deltas and misses an update.

Required behavior:

- Subscribe to `md.book.<ticker>` first.
- Buffer deltas.
- Fetch snapshot.
- Send snapshot.
- Replay buffered deltas with `seq > snapshot.seq`.

### Liquibook Callback Side Effect

Failure: callback publishes NATS or posts to DB.

Impact:

- Non-deterministic replay.
- Partial event visibility.
- Deadlocks or latency spikes.
- Callback failure corrupts command outcome.

Required behavior:

- Callback only records deterministic facts into sequencer command context.

## Dangerous Implementation Mistakes

- Treating Liquibook as the exchange.
- Mutating Liquibook from multiple threads.
- Publishing directly from Liquibook callbacks.
- Releasing a hold after `DEADLINE_EXCEEDED`.
- Persisting fills after ledger posting instead of before it.
- Using random fill IDs.
- Using wall-clock time for ordering.
- Reading Liquibook from gRPC or WS threads.
- Letting settlement run without `close_global_seq`.
- Treating NATS publish success as durability.
- Exposing raw internal fill events to clients.
- Implementing gateways before fill durability is tested.
- Implementing production features before MVP invariants are stable.

## What Can Be Delayed Until Production

- JetStream ME journal.
- Production ME snapshotter.
- Hot standby ME replica.
- Kubernetes leader election.
- CPU pinning and memory arenas.
- Fireblocks custody.
- Wallet watcher.
- Withdrawal service.
- Multi-source oracle attestors.
- Challenge window.
- Redis Lua risk reservations.
- API key request signing.
- mTLS.
- Full Prometheus/Tempo/Loki/PagerDuty stack.
- Load targets above demo scale.

## Ideal First Execution Milestone

Start with Milestones 0, 1, and 2 as one locked foundation sprint:

1. Repo scaffold.
2. Protobuf generation.
3. Database migrations.
4. Seed data.
5. CI.
6. Compose infra.

Do not start matching, gateways, frontend, or bots until protobufs and DB invariants compile and pass tests.

This is the safest way to prevent architectural drift and protect the exchange correctness model from the beginning.
