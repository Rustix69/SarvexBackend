# Milestone Updates

## Trade Bots (Deployed and Verified)

- Added a dedicated Rust `trade-bots` service that defaults to 30 deterministic demo bots and clamps configuration to 20-50 bots.
- Bots authenticate through the existing demo login route, receive idempotent demo funding, and submit orders through `gw-rest` so risk, holds, matching, fills, ledger posting, and event publication are exercised normally.
- Added deterministic demo risk-limit seeds for `u_bot_001` through `u_bot_050`.
- Added passive bid/ask book seeding for open binary and scalar contracts, followed by crossing IOC orders to create visible fills and market-data activity.
- Added configurable gateway URL, bot count, funding, interval, rounds, and ticker filters through environment variables.
- Added bounded passive depth configuration with `BOT_BOOK_LEVELS` (default `2`) so a large contract catalog does not consume all demo collateral.
- Added a bot health endpoint through the shared Rust runtime and exposed it in Docker Compose on port `18090`.
- Deployed from the pushed Rust backend to the live EC2 `sarvex-rust` Compose project.
- Verified all 14 services are running and the public health endpoint returns HTTP 200 with zero non-running services.
- Verified live PostgreSQL activity after rollout: 700 bot orders and 400 bot fills.

## Phase 09 (Rust REST Gateway Foundation) - In Progress

- Replaced the REST gateway health-only implementation with protobuf-backed delegation to refdata, order-router, ledger, and position services.
- Added demo login tokens using the existing `demo.<base64-user-id>` convention; protected endpoints require a bearer token.
- Added market listing, contract lookup, fill replay, order submit/list/get/cancel, balance/history, positions, open interest, and demo deposit routes.
- Added required `Idempotency-Key` enforcement at the REST boundary for order mutations; domain idempotency remains owned by order-router/ledger.
- Added bounded upstream RPC timeouts and gRPC-to-HTTP error mapping.
- Added explicit JSON conversion for protobuf enums, timestamps, contracts, orders, fills, balances, positions, and ledger history.
- Wired all gateway upstream addresses and service dependencies into the Rust Compose stack.
- WebSocket delivery, orderbook snapshot delegation, persistent gateway idempotency, and production authentication remain deferred.

## Phase 08 (Position Consumer, Gap Replay, and Read Model) - In Progress

- Added migration `000006_position_consumer` for durable consumer offsets, applied-fill deduplication, and position cost/PnL columns.
- Implemented `position-svc` as a separate gRPC/HTTP service with position, contract-position, and open-interest read APIs.
- Added NATS execution-fill consumption on `exec.fills.*` with reconnect handling.
- Added transactional fill application: applied-fill identity, maker/taker position deltas, and consumer offset advance commit together.
- Added gap detection and cursor-based replay through `order-router.ListFills`, including non-advancing cursor protection.
- Preserved NATS as a delivery mechanism only; PostgreSQL fill facts remain the replay source of truth.
- Added unit coverage for position side/action sign mapping and event payload compatibility.
- Average cost and realized/unrealized PnL remain read-model placeholders until mark/settlement semantics are implemented.
- Live PostgreSQL/NATS integration and end-to-end matching remain pending because the C++ me-core gRPC server is not yet available in the new compose stack.

## Phase 07 (Durable NATS Execution Event Spine) - In Progress
- Added `000005_execution_events` with a PostgreSQL-backed execution-event outbox keyed by immutable fill/event ID and ordered by `global_seq`.
- Fill persistence now writes the execution event envelope in the same transaction as the fill fact, order updates, and fill-posting outbox row.
- Added a retrying order-router publisher using NATS Core: it publishes `exec.fills.<ticker>` only from committed outbox rows and marks rows posted only after publish plus client flush.
- Publisher retries transient connection, database, and publish failures without making NATS the source of truth; duplicate delivery remains possible and requires consumer idempotency by event ID.
- Added a local NATS service with monitoring port to the Rust Compose stack.
- Added subject and envelope tests in `sarvex-events`.
- Position consumption and gap replay are implemented in Phase 08; JetStream durability and production event retention remain intentionally deferred.

## Phase 06 (Fill Durability and Event Contract Foundation) - In Progress
- Added the Rust `orders` migration with order state, immutable fill facts, and a transactional `fill_posting_outbox`.
- Fill persistence records maker/taker order and user facts, hold IDs, side/action pairs, prices, quantities, fees, and matching sequence metadata.
- The order-router persists each fill and its posting-outbox row in the same PostgreSQL transaction as order fill counters/status updates.
- Added replay-oriented `ListFills` support ordered by `global_seq` with optional ticker and sequence-range filters.
- Replaced the Rust event crate's Redis-style stream names with the architecture's NATS subject contract: execution, ledger, market-data, settlement, audit, user execution, user fills, book, trade, and ticker subjects.
- Added versioned event envelopes with subject, event ID, global sequence, contract sequence, timestamp, and JSON round-trip tests.
- Ledger fill posting and NATS publisher workers are intentionally not claimed complete yet; the destination-account seed and live me-core fill integration must be verified before those workers can safely commit holds or publish facts.

## Phase 05 (Rust Order-Router Orchestration Foundation) - In Progress
- Replaced the health-only Rust `order-router` shell with a tonic `OrderRouter` server for submit, lookup, list-orders, and replay/list-fills paths.
- Submit flow now inserts `PENDING` before downstream work, enforces `(user_id, client_order_id)` idempotency, validates refdata state, calls risk, places a deterministic ledger hold, and submits through the typed me-core client.
- Matching `OutcomeUnknown` preserves `PENDING` and the hold for reconciliation; queue-full and explicit matching rejection release the full uncommitted hold before becoming terminal rejection.
- Added IOC no-fill hold release and persisted maker/taker fill updates through a single database transaction.
- Cancel now delegates to me-core with unknown-outcome handling, marks terminal state only after engine confirmation, and releases hold remainder through an idempotent worker path.
- Amend now supports safe price/count changes when required collateral remains unchanged; collateral-changing amendments return `HOLD_RECONCILIATION_REQUIRED` until an atomic hold-adjustment contract exists.
- Added Compose wiring and `000004_orders` migration for the new service.
- Full Rust formatting, compilation, strict Clippy, and workspace tests pass; live PostgreSQL and cross-language matching integration remain pending.

## Phase 04 (me-core Boundary and Liquibook Integration Foundation) - In Progress
- Added the `sarvex-me-client` Rust crate as the typed client boundary for the frozen `MatchingEngine` protobuf service.
- Implemented typed add-book, submit, cancel, amend, close-book, and book-snapshot calls using tonic without moving matching or state ownership into Rust.
- Preserved the frozen Liquibook flag mapping for IOC, FOK, post-only, and reduce-only orders.
- Added explicit timeout semantics: local timeout and unavailable/deadline responses remain `OutcomeUnknown`; they are never treated as terminal order rejection. `RESOURCE_EXHAUSTED` remains distinguishable as a pre-enqueue queue-full rejection.
- Added client tests for flag mapping, side/action determinism, unknown outcomes, and queue-full classification.
- Wired `me-core-adapter` to initialize the typed client from `ME_CORE_ADDR` and `ME_CORE_TIMEOUT_MS` while retaining its health endpoint.
- Added the adapter to `services/docker-compose.yml` without moving matching ownership out of the active C++ process.
- Fixed the shared Rust health runtime JSON response to serialize its `Arc<str>` service name correctly under the current serde version.
- Rust formatting, workspace compilation, strict Clippy, and all workspace tests pass.
- Remaining Phase 04 work: expose the frozen MatchingEngine gRPC server from the preserved C++ Liquibook process, then add cross-language submit/cancel/snapshot integration tests. No matching state, ledger posting, NATS publishing, or replay logic was moved into the adapter.

## Phase 10 (Fill Ledger Posting and Terminal Hold Reconciliation) - In Progress

- Added an order-router fill-posting worker that drains `orders.fill_posting_outbox` after fill persistence.
- Each maker/taker hold commit uses deterministic `fill:<fill_id>:<party>` idempotency keys and posts collateral to `LIAB:HOUSE:UNSETTLED_TRADES:<ticker>`.
- Fill posting retries after ledger/refdata/timeouts and marks a fill posted only after both parties commit successfully.
- Added terminal-order remainder release based on the order hold amount minus all persisted fill collateral requirements.
- Ledger now creates destination accounts transactionally before commit-hold postings.
- Remaining work: live ledger/outbox integration tests and production settlement consumption of unsettled-trade accounts.

## Phase 11 (WebSocket Event Gateway) - In Progress

- Replaced the WebSocket health-only shell with an Axum WebSocket gateway backed by NATS.
- Added public market-trade subscriptions and authenticated private-fill subscriptions.
- Private events are filtered by user identity and do not expose counterparty IDs or orders.
- Added connection, subscription, ping/pong, malformed-message, and NATS-unavailable responses.
- Added bounded per-connection event buffering and subscription task cleanup on disconnect.
- Wired `gw-ws` into Compose on port `18082`.
- Snapshot-buffer-replay and book snapshot delegation remain pending until the matching-engine gRPC server is available.

## Phase 03 (Risk and Pre-Trade Foundation) - Completed
- Added risk persistence for user limits, per-contract position limits, working-order summaries, and the position read model used by pre-trade checks.
- Implemented Rust `risk-svc` over the frozen protobuf contract with `PreTradeCheck`, `GetUserLimits`, and transactional `UpdateUserLimits`.
- Risk now obtains contract state and limits through `refdata-svc` gRPC, preserving service ownership boundaries.
- Added validation for contract state, side/action combinations, positive limit prices, price bounds, tick alignment, maximum order size, binary/scalar hold requirements, user notional limits, and projected position limits.
- Added deterministic devnet risk-limit seeds and Compose migration/service wiring.
- Added unit coverage for binary buy/sell hold formulas, signed position deltas, and invalid side/action combinations.
- Rust formatting, compilation, strict Clippy, and workspace tests pass. PostgreSQL integration and Docker bring-up remain environment-blocked by unavailable local PostgreSQL/Docker daemon access.

## Phase 02 (Ledger and Hold Lifecycle Foundation) - Completed
- Added the `ledger` PostgreSQL migration with accounts, transactions, append-only entries, deferred balanced-transaction enforcement, event outbox, holds, hold operations, and the user-balance view.
- Implemented the Rust `ledger-svc` gRPC service for `PostTransaction`, `PlaceHold`, `ReleaseHold`, `CommitHold`, `GetBalance`, `GetAccountHistory`, and demo `AdminCreditDeposit`.
- Enforced idempotency at transaction and hold-operation boundaries using the frozen protobuf request fields.
- Added deterministic account locking order, non-negative user cash/holds enforcement, hold remaining-amount checks, atomic ledger/outbox writes, and graceful gRPC/HTTP health startup.
- Added the ledger migration to the Rust Compose bring-up and exposed ledger gRPC/health ports for local integration.
- Added unit coverage for balanced entries, invalid entries, non-negative account classification, and pre-database rejection of unbalanced transactions.
- Rust formatting, compilation, strict Clippy, and workspace tests pass. PostgreSQL integration and Docker bring-up remain environment-blocked by unavailable local PostgreSQL/Docker daemon access.

## Phase 01 (Rust Backend Workspace and Refdata Foundation) - Completed
- Archived the existing Go/C++ backend during the initial rebuild; the obsolete legacy application code has now been removed while `frontend/` remains unchanged.
- Retained the frozen protobuf contracts under root `proto/` and the Liquibook source under `third_party/liquibook/` because the active Rust and C++ builds still depend on them.
- Created the new Rust workspace under `services/` with shared crates for protobuf contracts, domain types, PostgreSQL access, events, and service runtime health endpoints.
- Added compilable Rust service boundaries for `gw-rest`, `gw-ws`, `order-router`, `risk-svc`, `position-svc`, `refdata-svc`, `oracle-svc`, `settlement-svc`, `audit-svc`, `admin-svc`, and `me-core-adapter`.
- Added the planned `ledger-svc` health-only service boundary.
- Added Rust protobuf generation sourced from the frozen root `proto/` files without changing protobuf semantics.
- Implemented SQLx-backed `refdata-svc` gRPC listing, lookup, state filtering, cursor pagination, event lookup, and transactional state transition support.
- Implemented the Phase 01 Axum REST gateway endpoints `GET /v1/markets` and `GET /v1/markets/:ticker`.
- Added clean Phase 01 refdata migration and deterministic seed data with resolved TASI, Nikkei, TTF, and 2028 nominee values.
- Added a Phase 01 Docker Compose stack for PostgreSQL, refdata migration/seed, refdata service, and REST gateway.
- Added a vendored `protoc` build dependency so Rust protobuf generation is reproducible without a system compiler.
- Added the workspace lockfile and completed `cargo fmt --check`, `cargo check --workspace`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace`.
- Confirmed `npm run build --prefix frontend` still passes and no frontend source files were modified.
- Docker image verification remains environment-blocked because this user cannot access `/var/run/docker.sock` and the Docker Compose plugin is unavailable; the Dockerfiles and compose configuration are committed for CI/host execution.

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

## MVP Backend E2E Validation - Completed
- Added repeatable live E2E test:
  - `pkg/e2e/mvp_e2e_test.go`
  - Run with: `go test -tags=e2e ./pkg/e2e -v`
- Verified full MVP lifecycle against Docker Compose services:
  - admin deposits
  - REST login
  - REST maker/taker order submission
  - matching fill creation
  - fill persistence
  - fill ledger posting
  - contract close
  - admin oracle force-resolution
  - settlement payout
  - final contract state `SETTLED`
- Fixed E2E blockers found during validation:
  - Added concrete MVP `MatchingEngine` gRPC server for compose/runtime testing.
  - Wired `me-core` compose service to expose the MVP matching role on gRPC.
  - Fixed refdata scanner handling for nullable contract text fields.
  - Fixed refdata scanner handling for nullable `close_global_seq`.
  - Increased order-router matching RPC timeout for compose/gRPC connection startup.
  - Fixed fill posting escrow account from demo placeholder to `UNSETTLED_TRADES:<ticker>`.
  - Fixed maker order fill update enum assignment and stopped ignoring its SQL error.
  - Fixed settlement catch-up prerequisite to require position catch-up through last fill at or before close.
- Validation completed:
  - `go test ./...` passed
  - `go test -tags=e2e ./pkg/e2e -v` passed
  - Docker Compose stack healthy on alternate host ports

## MVP Demo Liquidity Simulation - Completed
- Added frontend/demo liquidity support so markets do not look empty during investor walkthroughs.
- Added simulator command in `cmd/demo-sim/main.go`:
  - creates `u_sim_001...u_sim_N` demo users and risk limits
  - funds simulated users through `ledger-svc.AdminCreditDeposit`
  - logs in through `POST /v1/auth/login`
  - submits maker/taker orders through real `POST /v1/orders`
  - generates visible order book depth and recent fills without direct fill insertion
  - can reopen the matching book with `-reset-book` after close/settlement demos
  - supports one-shot and continuous modes
- Added runner script:
  - `scripts/run-demo-sim.sh`
  - default REST: `http://localhost:19080`
  - default ledger gRPC: `localhost:15062`
  - default matching gRPC: `localhost:15064`
  - default Postgres DSN: `postgres://sarvaex:sarvaex@localhost:15432/sarvaex?sslmode=disable`
- Added REST frontend reads in `cmd/gw-rest/main.go`:
  - `GET /v1/markets?state=OPEN&limit=10`
  - `GET /v1/markets/{ticker}/orderbook?depth=10`
  - test fixture markets are hidden by default unless `include_test=true`
- Completed MVP order book snapshot support in `pkg/m3svc/matching_server.go`:
  - aggregates resting BUY orders into bid levels
  - aggregates resting SELL orders into ask levels
  - sorts bids descending and asks ascending
  - honors snapshot depth
- Live verification completed:
  - `go run ./cmd/demo-sim -users 12 -rounds 2 -fund-usdc 0 -interval 100ms -reset-book` populated the book
  - `GET /v1/markets/RBI-JUN26-CUT25/orderbook?depth=5` returned bid/ask depth
  - `GET /v1/markets/RBI-JUN26-CUT25/fills?limit=5` returned simulator-generated fills
- Validation completed:
  - `go test ./...` passed
  - `go test -count=1 -tags=e2e ./pkg/e2e -v` passed after simulator/orderbook changes

## Frontend MVP Trading Shell - Started
- Built the first investor-demo frontend in `frontend/src/App.jsx` and `frontend/src/App.css` using the light card-based market style from the provided references.
- Implemented dashboard flow:
  - top navigation with demo user selector
  - category bar
  - market cards from `GET /v1/markets?state=OPEN&limit=12`
  - Polymarket-style card rows with Yes/No pricing
- Implemented market detail flow:
  - chart panel
  - contract rows
  - order ticket
  - order book from `GET /v1/markets/{ticker}/orderbook?depth=12`
  - recent trades from `GET /v1/markets/{ticker}/fills?limit=30`
  - portfolio panel using balance, positions, and orders endpoints
- Implemented frontend trading actions:
  - demo login through `POST /v1/auth/login`
  - demo self-funding through `POST /v1/demo/deposits/credit`
  - order submission through `POST /v1/orders` with `Idempotency-Key`
- Backend gateway additions for frontend support:
  - CORS handling
  - `POST /v1/demo/deposits/credit`
  - `POST /v1/admin/deposits/credit`
- Vite proxy added:
  - frontend calls `/api/*`
  - proxy forwards to `http://127.0.0.1:19080`
- Validation completed:
  - `npm run lint` passed
  - `npm run build` passed
  - `go test ./...` passed
  - Vite dev server served `http://127.0.0.1:5173/`
  - Vite `/api/v1/markets?state=OPEN&limit=2` returned live backend data
  - demo deposit endpoint verified with bearer token
  - simulator repopulated live orderbook with bid/ask depth

## Phase 12: C++ me-core and Market Data Bridge - Implemented
- Added a new C++ `services/me-core/` target around the preserved Liquibook source.
- Implemented the frozen `MatchingEngine` gRPC surface:
  - add/close book
  - submit/cancel/amend order
  - book snapshot
  - replayable execution stream
- Added `SarvaOrder` with SarvEX-owned order ID, user ID, hold ID, side, action,
  quantity, and price metadata while Liquibook remains responsible for matching
  and price-level tracking.
- Added a single-writer sequencer queue with bounded capacity and explicit
  `RESOURCE_EXHAUSTED` versus `DEADLINE_EXCEEDED` behavior.
- Added immutable execution facts for accepted, rejected, fill, cancelled,
  amended, and book-delta events with global and per-contract sequence values.
- Added in-memory replay history and live subscribers for `StreamExecutions`.
- Added Liquibook-backed aggregate snapshots and book delta generation.
- Added `marketdata-svc`, which reconnects to the C++ execution stream and
  publishes durable-contract-compatible NATS envelopes to:
  - `md.trade.<ticker>`
  - `md.book.<ticker>`
- Added C++ me-core and market-data services to Docker Compose wiring.
- Added an ignored cross-language Rust gRPC smoke test that submits maker/taker
  orders to the C++ server, verifies the fill and snapshot, and replays the fill.
- Updated order-router to idempotently ensure a matching book exists from
  refdata before submitting the first order.

### Validation
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo check --workspace --all-targets`
- `cargo test --workspace`
- `git diff --check`

### Environment limitation
- The local environment has no Docker daemon and no installed C++ gRPC/protobuf
  headers, so the C++ compile and live cross-language test remain to be run on
  the Docker-enabled EC2/build host:
  - `docker compose -f services/docker-compose.yml build me-core marketdata-svc`
  - `ME_CORE_TEST_ADDR=http://127.0.0.1:15054 cargo test -p sarvex-me-client --test cross_language_smoke -- --ignored`

## Phase 13-14: Recovery, Settlement, Auth, Retention, and Load Readiness - Implemented
- Added `sarvex-auth` with an explicit authentication mode:
  - `AUTH_MODE=demo` preserves the unsigned `demo.<base64url(user_id)>` token for local/demo use.
  - `AUTH_MODE=jwt` issues and verifies HS256 JWTs with issuer, audience, expiry, and a required `JWT_SECRET` of at least 32 bytes.
  - JWT login is gated by `AUTH_LOGIN_SECRET` until a real identity provider is connected.
- Added persistent gateway idempotency migration `000008_gateway_idempotency`.
  - REST submit, cancel, and demo deposit mutations now replay stored responses.
  - Reusing a key with a different request hash returns `IDEMPOTENCY_KEY_REUSED`.
- Added WebSocket market snapshot-buffer-replay:
  - subscribes to `md.book.<ticker>` before requesting the matching-engine snapshot
  - buffers deltas during the snapshot call
  - emits the snapshot, replays ordered deltas newer than the snapshot sequence, then streams live deltas
- Added demo me-core command journaling and restoration:
  - `ME_CORE_JOURNAL_PATH` stores length-delimited AddBook/CloseBook/Submit/Cancel/Amend commands
  - the journal is replayed before accepting requests after restart
  - Docker Compose persists it in the `me_core_state` volume
- Added oracle service logic:
  - attestation upsert
  - quorum and challenge-window checks
  - conflicting-attestation detection and DISPUTED state
  - finalized-resolution event publication
  - admin force-resolution path with required identity and justification
- Added settlement service logic:
  - checks closed/resolving state, close sequence, unposted fills, active orders, and position consumer convergence
  - creates durable payout intents before ledger calls
  - uses idempotent ledger payout transactions
  - computes binary and scalar payouts with checked integer arithmetic
  - snapshots escrow before posting and performs a deterministic rounding sweep
  - marks the contract SETTLED only after all payout work completes
- Added JetStream-capable `EventPublisher` for `md.>`, `exec.>`, `oracle.>`, `settlement.>`, and `ledger.>` subjects.
  - core NATS remains the default; `EVENT_RETENTION=jetstream` enables file-backed retention
  - NATS Compose now starts with JetStream enabled
- Added `/metrics` Prometheus text endpoints to runtime, REST, and WebSocket gateways.
- Added `services/loadtest`, a concurrent me-core gRPC load harness reporting p50/p95/p99 latency.
- Added oracle and settlement services to Docker Compose with their service dependencies.

### Validation
- `cargo fmt --all`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `git diff --check`

### Remaining deployment-only verification
- Docker Compose build and migration execution on a Docker-enabled host.
- C++ me-core build and ignored Rust/C++ cross-language smoke test.
- EC2 load test, JetStream retention inspection, and production metrics scraping.
