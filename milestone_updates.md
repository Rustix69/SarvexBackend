# Milestone Updates

## Milestone 0 (Repo Bootstrap) - Completed
- Created monorepo foundation directories: `services/`, `proto/`, `db/`, `scripts/`, `pkg/`, `web/`, `.github/workflows/`.
- Added root bootstrap files: `.env.example`, `docker-compose.yml`, `Makefile`, `ROADMAP.md`.
- Added CI workflow: `.github/workflows/ci.yml` running `make build` and `make test` plus compose validation.
- Added infra-only compose services for Milestone 0: `postgres`, `nats`, `redis` with health checks.
- Added bootstrap scripts: `scripts/proto-gen.sh`, `scripts/run-demo.sh`, `scripts/reset-demo.sh`, `scripts/seed-demo-data.sh`, `scripts/record-demo.sh`.
- Added migration runner scaffold: `scripts/migrate.sh` and wired `make migrate` / `make migrate-down`.
- Added Buf scaffolding for upcoming protobuf work: `buf.yaml`, `buf.gen.yaml`.
- Added proto scaffold and placeholders: `proto/README.md`, `proto/sarvex/v1/.gitkeep`.
- Added `me-core` CMake shell and boot executable scaffold: `services/me-core/CMakeLists.txt`, `services/me-core/src/main.cpp`.
- Added directory-tracking placeholders (`.gitkeep`) across service, package, migration, and web subdirectories.
- Added seed placeholders: `db/seed/contracts.sql`, `db/seed/demo_users.sql`.
- Added scaffold documentation files: `services/README.md`, `db/README.md`, `pkg/README.md`, `web/README.md`.
- Validated scaffold commands: `make build`, `make test`, and `make proto` (no proto files yet, expected skip).
- Attempted `make run`; compose bootstrap is correct but local host port `6379` was already in use, so infra startup requires a local port override in `.env`.
- Verified end-to-end bring-up with port overrides: `POSTGRES_PORT=15432 NATS_PORT=14222 NATS_MONITOR_PORT=18222 REDIS_PORT=16379 make run`.

## Milestone 01 (Protobuf Freeze) - Completed
- Replaced proto placeholder with real contracts under `proto/sarvex/v1/`:
  `audit.proto`, `common.proto`, `ledger.proto`, `marketdata.proto`, `match.proto`,
  `oracle.proto`, `order.proto`, `position.proto`, `refdata.proto`, `risk.proto`, `settlement.proto`.
- Updated `buf.yaml` and `buf.gen.yaml` to current Buf config format and aligned generation outputs with `services/me-core/gen`.
- Updated `scripts/proto-gen.sh` to:
  prefer `${HOME}/go/bin/buf` when present,
  use repo-local cache (`.cache/`) for stable execution,
  and keep C++ output path consistent with `services/me-core/gen`.
- Removed old proto placeholder file `proto/sarvaex/v1/.gitkeep` in favor of real proto package content.
- Generated and committed Go bindings under `gen/go/sarvex/v1/`.
- Generated and committed C++ bindings under `services/me-core/gen/sarvex/v1/`.
- Added typed gRPC stub-service registration entrypoint at `cmd/proto-stub-server/main.go` covering:
  `Ledger`, `MatchingEngine`, `Oracle`, `OrderRouter`, `Position`, `RefData`, `Risk`, `Settlement`.
- Added Go module/dependency lock (`go.mod`, `go.sum`) and verified the stub server compiles.
- Validated Milestone 01 contract checks from `planning.md`:
  `MeFill` includes maker/taker facts + holds + fees + seqs,
  `CloseBookResponse` includes `close_global_seq`,
  `Position.ListPositionsByContract` includes `min_global_seq`,
  `OrderRouter.ListFills` supports replay fields,
  Ledger hold APIs include idempotency keys on place/release/commit.

## Milestone 02 (Database Migrations and Seeds) - Completed
- Added full schema migration set:
  - `db/migrations/000001_milestone2_init.up.sql`
  - `db/migrations/000001_milestone2_init.down.sql`
- Implemented all required service schemas:
  `refdata`, `users`, `ledger`, `orders`, `risk`, `position`, `oracle`, `settlement`, `audit`.
- Implemented required persistence invariants:
  - ledger balanced transaction constraint trigger (`ledger.assert_tx_balanced`)
  - append-only protection on `ledger.entries` (update/delete blocked)
  - `orders.orders` unique `(user_id, client_order_id)`
  - `orders.fills.global_seq` unique and `(ticker, ticker_seq)` unique
  - `orders.fill_posting_outbox` table
  - `ledger.hold_operations` table
  - `position.consumer_offsets` and `position.applied_fills` tables
  - `settlement.settlement_payouts.idempotency_key` unique
- Replaced seed placeholders with real seed data:
  - `db/seed/contracts.sql`
  - `db/seed/demo_users.sql`
  - `db/seed/house_accounts.sql`
- Wired executable seed script (`scripts/seed-demo-data.sh`) to apply all seed SQL via `psql`.
- End-to-end validation performed against local Postgres (compose on alternate ports):
  - migration `up` succeeded
  - migration `down` then `up` succeeded
  - seed execution succeeded
  - required table/unique checks succeeded
  - unbalanced ledger entry was rejected by trigger (expected)

## Milestone 03 (Service Skeleton and Compose Bring-Up) - Completed
- Added shared service runtime scaffolding:
  - `pkg/m3svc/config.go` for config loading from env
  - `pkg/m3svc/app.go` for gRPC skeleton startup, dependency checks (Postgres/NATS), and health/readiness endpoints
  - `cmd/svc-server/main.go` generic service entrypoint with role-based gRPC registration
- Added gateway skeletons:
  - `cmd/gw-rest/main.go` with readiness/liveness and placeholder REST endpoints
  - `cmd/gw-ws/main.go` with readiness/liveness and WebSocket welcome handshake
- Added Dockerfiles for all services:
  - `services/*/Dockerfile` across admin, audit, gw-rest, gw-ws, ledger, me-core, oracle, order-router, position, refdata, risk, settlement
- Updated `docker-compose.yml` to full Milestone 03 topology:
  - infra + migrations + all backend services + gateways
  - explicit dependency ordering and health checks
  - service env wiring for DB/NATS
- Updated `.env.example` with service port mappings.
- Updated `Makefile`:
  - `build` -> `go build ./...`
  - `test` -> `go test ./...`
  - `run` -> full compose stack with build
- Updated `services/me-core/src/main.cpp` and `services/me-core/Dockerfile` so `me-core` stays up as a long-running service in compose.
- Validation completed:
  - `go mod tidy` successful
  - `go build ./...` and `go test ./...` successful
  - `docker compose config` successful
  - full compose bring-up successful with local port overrides
  - all service containers healthy; `migrations` exits `0` as expected one-shot job

## Milestone 04 (Ledger and Hold Lifecycle) - Completed
- Implemented real `ledger-svc` gRPC behavior in `pkg/m3svc/ledger_server.go`:
  - `PostTransaction`
  - `PlaceHold`
  - `ReleaseHold`
  - `CommitHold`
  - `GetBalance`
  - `GetAccountHistory`
  - `AdminCreditDeposit`
- Wired ledger role registration to use the concrete server with DB pool.
- Implemented core ledger invariants in service logic:
  - idempotent transaction creation (`ledger.transactions.idempotency_key`)
  - lazy account creation for user and house codes
  - deterministic account locking order
  - running balance + account sequence writes
  - non-negative enforcement for user `CASH` and `HOLDS`
  - hold lifecycle accounting (`ACTIVE` -> `CLOSED`)
  - idempotent hold operations via `ledger.hold_operations`
  - outbox emit to `ledger.ledger_event_outbox`
- Added Milestone 04 tests in `pkg/m3svc/ledger_server_test.go`:
  - admin deposit/balance flow
  - place-hold idempotency + release flow
  - insufficient funds rejection on hold placement
- Validation completed:
  - `go test ./pkg/m3svc -v` passed against local Postgres
  - `go test ./...` passed

## Milestone 05 (Refdata and Risk MVP) - Completed
- Implemented real `refdata-svc` handlers in `pkg/m3svc/refdata_server.go`:
  - `GetContract`
  - `ListContracts`
  - `TransitionState`
  - `UpsertContract`
  - `GetEvent`
- Implemented real `risk-svc` handlers in `pkg/m3svc/risk_server.go`:
  - `PreTradeCheck`
  - `GetUserLimits`
  - `UpdateUserLimits`
- Wired `refdata` and `risk` role registration to concrete servers with shared DB pool in `pkg/m3svc/app.go`.
- Refdata implementation details:
  - contract enum/state mapping between proto and DB enums
  - contract state transitions persisted to `refdata.contract_state_history`
  - list filtering (`state`, `series_ticker`) and cursor pagination
  - typed conversion of `settlement_rule` JSON to protobuf struct
- Risk implementation details:
  - binary hold formulas (`BUY` and `SELL`) using integer math
  - scalar LONG/SHORT hold formulas using bounds and multiplier
  - sanity checks for contract state, price range, tick alignment, and quantity limits
  - projected position check via `position.positions` + `risk.working_orders_summary`
  - per-contract override from `risk.contract_position_limits`
- Added Milestone 05 tests in `pkg/m3svc/milestone5_test.go`:
  - refdata upsert/get/transition/list lifecycle
  - risk pre-trade approve and reject flows (position limit)
- Validation completed:
  - `go test ./pkg/m3svc -v` passed against local Postgres
  - `go test ./...` passed

## Milestone 06 (`me-core` Liquibook Scaffold) - Completed
- Implemented `me-core` scaffold layer under `services/me-core/src/mecore/`:
  - `sarva_order.h` (`SarvaOrder` with Liquibook order interface)
  - `book_state.h` (`BookState` with one `DepthOrderBook` per ticker)
  - `shard_state.h` (`ShardState` ownership maps + sequencer counters scaffold)
  - `listener_bridge.h/.cpp` (Order/Trade/Depth callback bridge shell)
  - `me_core_engine.h/.cpp` (`add_book` + `get_book_snapshot` placeholder)
- Updated executable startup (`services/me-core/src/main.cpp`) to instantiate engine, create a demo book, and keep service process alive.
- Updated CMake to Milestone 06 layering:
  - `liquibook_headers` interface target
  - `me_core_proto_headers` include target
  - `me_core_lib` static library for core engine/listener code
  - `me-core` executable linked to `me_core_lib`
- Added protobuf compatibility shim:
  - `services/me-core/src/mecore/proto_compat.h`
  - Uses generated proto enums when compatible headers are available; falls back to local contract-kind enum to keep build green when proto runtime/header versions are mismatched.
- Updated `services/me-core/Dockerfile` build deps for C++ scaffold compilation.
- Validation completed:
  - `docker compose ... up -d --build me-core` succeeds
  - `me-core` container status: `Up`
  - migrations job exits `0` in compose lifecycle

## Milestone 07 (Sequencer, Matching, Events, Snapshots) - Completed
- Upgraded `services/me-core/src/mecore/me_core_engine.h/.cpp` from static scaffold to sequencer-driven engine:
  - command queue and worker thread
  - serialized single-writer command processing
  - typed command handlers for:
    - `AddBook`
    - `SubmitOrder`
    - `CancelOrder`
    - `CloseBook`
    - `GetBookSnapshot`
- Implemented sequence assignment before Liquibook mutation:
  - `global_seq` and `contract_seq` are assigned in sequencer path before `book.add()` / `book.cancel()`.
- Implemented matching command behavior:
  - post-only crossing pre-check and reject code
  - IOC/FOK scaffold integration
  - deterministic fill id generation (`ticker:contract_seq:fill_idx`)
  - close-book response fields (`close_global_seq`, `close_contract_seq`)
- Kept callback boundary clean:
  - listener bridge (`listener_bridge.*`) only appends in-memory events and does not perform network/DB I/O.
- Extended `main.cpp` startup flow to exercise sequencer submit path and print sequence evidence.
- Validation completed:
  - `docker compose --env-file .env.example up -d --build me-core` successful
  - me-core container status `Up`
  - startup log confirms sequence assignment on submit (`seq=1/1`)

## Milestone 08 (Order Router and Fill Durability) - Completed
- Implemented real `order-router` gRPC server in `pkg/m3svc/order_router_server.go`:
  - `SubmitOrder`
  - `CancelOrder`
  - `AmendOrder` (explicitly stubbed as `Unimplemented` per plan)
  - `GetOrder`
  - `ListOrders`
  - `ListFills`
- Wired role registration in `pkg/m3svc/app.go` so `SERVICE_ROLE=order-router` now uses DB/config-backed server instance.
- Added matching endpoint config in `pkg/m3svc/config.go`:
  - `MATCHING_ENGINE_ADDR` (default `me-core:50051`)
- Implemented Milestone 08 durability path in `SubmitOrder`:
  - inserts `orders.orders` row as `PENDING` before side effects
  - handles `(user_id, client_order_id)` duplicate path by returning existing order
  - runs refdata contract-state validation and risk pre-trade check
  - places ledger hold with deterministic idempotency key
  - submits to matching engine with bounded timeout
  - preserves `PENDING` + returns `ACK_UNKNOWN` when matching outcome is unknown (timeout/unavailable)
  - handles queue-full (`RESOURCE_EXHAUSTED`) with hold release + terminal reject
  - on accepts, persists fills and outbox rows transactionally with order status/fill counters update
- Implemented fill durability worker pass (`runFillPosterOnce`) using `orders.fill_posting_outbox` as source of truth:
  - drains pending outbox rows
  - posts hold commits idempotently via deterministic `fill:<fill_id>:<side>` keys
  - marks fill/outbox status as posted
- Implemented cancel path hold-release behavior:
  - reads remaining hold amount from ledger
  - releases remaining held funds idempotently
  - transitions order to `CANCELLED`
- Validation completed:
  - `go test ./...` passed after implementation

## Milestone 09 (NATS Event Spine and Consumers) - Completed
- Added event spine + consumers in `pkg/m3svc/milestone9_spine.go`:
  - `runLedgerOutboxPublisher`:
    - publishes `ledger.events` from durable `ledger.ledger_event_outbox`
    - publishes `ledger.balance.user.<user_id>` for users touched by each ledger tx
    - marks outbox rows `POSTED` only after publish attempt
  - `runPositionFillConsumer` + `applyPositionFill`:
    - subscribes to `exec.fills.*`
    - persists offset in `position.consumer_offsets`
    - detects sequence gaps (`global_seq > last+1`)
    - replays gaps through `OrderRouter.ListFills`
    - enforces idempotency via `position.applied_fills(fill_id)`
    - applies maker/taker position deltas + history writes transactionally
  - `runRiskFillConsumer`:
    - consumes `exec.fills.*`
    - decrements `risk.working_orders_summary` quantities on fills
  - `runAuditConsumer`:
    - consumes `exec.events` and `ledger.events`
    - persists into `audit.events` using `audit.event_seq_gen`
- Wired role worker startup in `pkg/m3svc/app.go`:
  - `ledger` starts ledger outbox publisher
  - `position` starts position fill consumer
  - `risk` starts risk fill consumer
  - `audit` starts audit consumer
- Extended order-router event publishing in `pkg/m3svc/order_router_server.go`:
  - order lifecycle -> `exec.events`
  - sanitized order user feed -> `exec.user.<user_id>`
  - fills -> `exec.fills.<ticker>`
  - per-user fill feed -> `exec.fills.user.<user_id>`
  - market trade/ticker feed -> `md.trade.<ticker>`, `md.ticker.<ticker>`
- Added concrete `position-svc` RPC implementation in `pkg/m3svc/position_server.go`:
  - `GetPosition`
  - `ListPositions`
  - `ListPositionsByContract` (supports `min_global_seq` filtering)
  - `GetOpenInterest`
- Validation completed:
  - `go test ./...` passed

## Milestone 10 (Gateways) - Completed
- Replaced `gw-rest` skeleton with Milestone 10 REST gateway implementation in `cmd/gw-rest/main.go`:
  - Demo auth/login endpoint:
    - `POST /v1/auth/login` returns bearer token (`demo.<base64(user_id)>`)
  - Auth middleware for private endpoints using bearer token parsing.
  - Idempotency middleware behavior for mutating endpoints:
    - requires `Idempotency-Key`
    - captures and replays cached response for duplicate key per `(user_id, path, key)`
  - Order endpoints backed by `order-router` gRPC:
    - `POST /v1/orders`
    - `GET /v1/orders`
    - `GET /v1/orders/{order_id}`
    - `POST /v1/orders/{order_id}/cancel`
  - Market endpoints:
    - `GET /v1/markets/{ticker}` (refdata contract)
    - `GET /v1/markets/{ticker}/fills` (replay/list fills)
  - Account endpoints:
    - `GET /v1/account/balance`
    - `GET /v1/account/history`
  - Position endpoint:
    - `GET /v1/positions`
  - Added gRPC error-to-HTTP mapping and request timeout boundaries.

- Replaced `gw-ws` skeleton with Milestone 10 WebSocket gateway implementation in `cmd/gw-ws/main.go`:
  - WS command protocol:
    - `{\"op\":\"auth\",\"token\":\"...\"}`
    - `{\"op\":\"subscribe\",\"channel\":\"market\",\"ticker\":\"...\"}`
    - `{\"op\":\"subscribe\",\"channel\":\"private\"}`
  - Private channel auth enforcement.
  - NATS bridges:
    - market: `md.trade.<ticker>`
    - private: `exec.user.<user_id>`, `exec.fills.user.<user_id>`, `ledger.balance.user.<user_id>`
  - Snapshot-buffer-replay flow for market subscription:
    - subscribes to deltas first
    - reads snapshot sequence (`MAX(global_seq)` per ticker from durable fills)
    - sends snapshot message
    - replays buffered deltas with `seq > snapshot.seq`
  - Backpressure handling:
    - bounded outbound queue per connection
    - closes connection with policy violation when queue overflows

- Validation completed:
  - `go test ./...` passed

## Milestone 11 (Close, Oracle, Settlement) - Completed
- Implemented Oracle service in `pkg/m3svc/oracle_server.go`:
  - `ProposeResolution`
  - `FinalizeResolution`
  - `GetResolution`
  - `AdminForceResolution`
- Oracle behavior implemented:
  - persists attestations + resolution state in `oracle.*` tables
  - finalized resolutions are idempotent
  - publishes `oracle.resolutions.finalized.<event_ticker>` on finalize

- Implemented Settlement service in `pkg/m3svc/settlement_server.go`:
  - `SettleContract`
  - `GetSettlement`
- Settlement behavior implemented:
  - enforces prerequisites:
    - `close_global_seq` must exist
    - all fills through close seq must be ledger-posted
    - no active (`PENDING`/`OPEN`/`PARTIAL`) orders remain
    - position consumer offset must be caught up through close seq
    - finalized oracle resolution required
  - transitions contract lifecycle:
    - `CLOSED -> RESOLVING -> SETTLED`
  - computes payouts using integer math:
    - binary YES/NO winner payout
    - scalar payout via bounds + multiplier
  - writes payout intents in `settlement.settlement_payouts`
  - posts idempotent ledger payouts with key `settlement:<ticker>:<user_id>`
  - executes rounding sweep posting path

- Implemented close coordination in `pkg/m3svc/refdata_server.go`:
  - enhanced `TransitionState` for `CONTRACT_STATE_CLOSED`:
    - calls `me-core.CloseBook` (best-effort) to fetch `close_global_seq`
    - persists `close_global_seq` and `close_at`
    - releases remaining holds for active orders on ticker with deterministic idempotency keys
    - marks active ticker orders terminal (`EXPIRED`)

- Worker wiring completed:
  - `pkg/m3svc/milestone9_spine.go` adds `runSettlementWorker`:
    - subscribes to `oracle.resolutions.finalized.*`
    - finds affected closed/resolving contracts
    - runs settlement automatically
  - `pkg/m3svc/app.go` starts settlement worker for `SERVICE_ROLE=settlement`
  - `pkg/m3svc/app.go` now wires concrete Oracle/Settlement servers with DB/NATS deps

- Validation completed:
  - `go test ./...` passed
