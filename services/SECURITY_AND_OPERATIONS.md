# Sarvex Rust Backend Security and Operations

## Authentication

The gateway uses `sarvex-auth`.

- `AUTH_MODE=demo` is the local/demo mode. Login returns the existing unsigned
  `demo.<base64url(user_id)>` token. It is identity labeling only and must not be
  exposed as production authentication.
- `AUTH_MODE=jwt` is the production code path currently implemented. It issues
  and verifies HS256 JWTs using `JWT_SECRET`, `JWT_ISSUER`, `JWT_AUDIENCE`, and
  `JWT_TTL_SECONDS`. `JWT_SECRET` must be at least 32 bytes. Users register with
  a user ID, email, and password; passwords are stored as Argon2id hashes in
  PostgreSQL and are never stored or returned in plaintext.
- API clients can authenticate with `X-API-Key: svx_live_<secret>`. Only the
  one-time create response contains the plaintext key. The database stores a
  SHA-256 hash, prefix, owner, scopes, expiry, and revocation timestamp.
- API keys are scoped. Trading mutations require `trading:write`, and WebSocket
  connections require `websocket:read`. Revoked or expired keys are rejected.
- REST and WebSocket gateways use the same credential conventions, so a token
  cannot be accepted by one gateway and rejected by the other because of
  different token parsing rules.

Password reset, email verification, MFA, and an external OIDC/RS256 issuer with
key rotation remain later hardening work. They do not change gateway ownership
or the Bearer-token/API-key contracts.

## Event retention

`EVENT_RETENTION=jetstream` makes `sarvex-events::EventPublisher` create/use the
file-backed `SARVEX_EVENTS` stream for `md.>`, `exec.>`, `oracle.>`,
`settlement.>`, and `ledger.>` subjects. The default `core` mode publishes to
core NATS and is suitable for local/demo runs where database outboxes and
matching-engine replay are the recovery source.

## Demo me-core recovery

Set `ME_CORE_JOURNAL_PATH` to a persistent path. The C++ me-core journals the
ordered command stream and replays it before serving gRPC requests. The Docker
Compose volume `me_core_state` is the demo persistence boundary. It is a flushed
command journal, not a production write-ahead log with group-commit/fsync and
checksums; production hardening should add those guarantees before relying on
it for a failover design.

## Load test

With me-core running, execute from `services/`:

```bash
ME_CORE_ADDR=http://127.0.0.1:50054 \
LOADTEST_ORDERS=10000 LOADTEST_CONCURRENCY=64 \
cargo run -p loadtest --release
```

The harness prints JSON with success count and p50/p95/p99 command latency.
