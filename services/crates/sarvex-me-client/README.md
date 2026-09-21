# sarvex-me-client

Typed Rust client boundary for the frozen `MatchingEngine` protobuf service.

This crate owns only transport concerns:

- protobuf request/response calls for add, submit, cancel, amend, close, and snapshot operations;
- the frozen Liquibook flag mapping (`IOC`, `FOK`, `POST_ONLY`, `REDUCE_ONLY`);
- timeout classification that preserves an unknown outcome;
- gRPC status classification for queue-full rejection.

It does not own order state, holds, ledger posting, event persistence, NATS publishing,
reconciliation, or replay. Those remain in the order-router, ledger, event, and recovery
boundaries described by the architecture.

`ME_CORE_ADDR` is intentionally configurable. The Phase 04 Rust adapter can start before
the preserved C++ process exposes its gRPC server, but no successful order command should
be claimed until that server is available.
