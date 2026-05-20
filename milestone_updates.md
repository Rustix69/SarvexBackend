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
