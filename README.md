# Sarvex

Sarvex contains the existing frontend and a Rust rebuild of the backend.

## Layout

- `frontend/` - existing Vite frontend.
- `services/` - new Rust backend workspace.
- `backend-old/` - archived Go/C++ backend, protobufs, Liquibook integration, database migrations, and deployment files.
- `planning.md` - implementation roadmap.
- `milestone_updates.md` - completed milestone notes.

The Rust rebuild preserves the existing service boundaries and uses the archived protobuf contracts as its initial internal API reference.

## Phase 01

From the repository root:

```bash
docker build -f services/refdata-svc/Dockerfile .
```

The local Rust toolchain is required for native development. The current phase implements the refdata service and public market listing/detail endpoints without changing the frontend.
