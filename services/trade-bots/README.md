# Sarvex Trade Bots

`trade-bots` is demo-only market activity infrastructure. It uses the public
REST gateway and therefore exercises authentication, risk checks, holds,
order routing, me-core matching, fill persistence, ledger posting, positions,
and market-data publication through the normal exchange path.

The service provisions deterministic users `u_bot_001` through `u_bot_050`.
The default is 30 active bots, two passive levels per side, and one buy plus
one sell IOC attempt per open market each round.

Useful environment variables:

- `BOT_COUNT`: 20-50, default `30`.
- `BOT_FUND_USDC`: idempotent demo funding per bot, default `100000`.
- `BOT_BOOK_LEVELS`: passive levels per side, default `2`.
- `BOT_INTERVAL_MS`: delay between rounds, default `1000`.
- `BOT_ROUNDS`: finite rounds for a test run; `0` means continuous.
- `BOT_TICKERS`: optional comma-separated allowlist of open tickers.

For a bounded local run against an already running stack:

```bash
GW_REST_URL=http://127.0.0.1:18080 \
BOT_COUNT=20 \
BOT_ROUNDS=2 \
cargo run --manifest-path services/Cargo.toml -p trade-bots
```
