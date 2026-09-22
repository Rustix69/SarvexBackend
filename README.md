# Sarvex

Sarvex contains the existing frontend and a Rust rebuild of the backend.

## Layout

- `frontend/` - existing Vite frontend.
- `services/` - new Rust backend workspace.
- `proto/` - frozen protobuf contracts used by the Rust services and C++ matching engine.
- `third_party/liquibook/` - Liquibook source used by `services/me-core`.
- `planning.md` - implementation roadmap.
- `milestone_updates.md` - completed milestone notes.

The Rust backend preserves the existing service boundaries and uses the frozen protobuf contracts as its internal API reference.

## Phase 01

From the repository root:

```bash
docker build -f services/refdata-svc/Dockerfile .
```

The local Rust toolchain is required for native development. The current phase implements the refdata service and public market listing/detail endpoints without changing the frontend.
