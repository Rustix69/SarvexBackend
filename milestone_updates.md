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
