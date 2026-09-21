# Sarvex me-core

`me-core` is the C++ matching-engine boundary. Liquibook owns only the in-memory
price-time book and matching algorithm. Sarvex owns the single-writer command
queue, order metadata, sequencing, execution facts, snapshots, and replay
stream around it.

Build locally on a host with gRPC and protobuf development packages:

```bash
cmake -S services/me-core -B services/me-core/build
cmake --build services/me-core/build --parallel
ME_CORE_LISTEN_ADDR=127.0.0.1:50054 services/me-core/build/me-core
```

The live subscriber replay buffer is bounded and in-memory, while the demo can
also restore ordered commands from `ME_CORE_JOURNAL_PATH` before accepting
requests. Set that variable to a persistent file to restore books after a
process restart. The journal can be flushed and fsynced when
`ME_CORE_JOURNAL_FSYNC=true`, but is not yet a checksummed/group-commit
production WAL. A production journal/snapshot writer must
consume the same `StreamExecutions` sequence without being called from a
Liquibook callback.

Cross-language smoke test, with the C++ process running:

```bash
ME_CORE_TEST_ADDR=http://127.0.0.1:50054 \
  cargo test --manifest-path services/Cargo.toml \
  -p sarvex-me-client --test cross_language_smoke -- --ignored
```
