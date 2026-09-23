# Sarvex Trade Bots

`trade-bots` is demo-only market activity infrastructure. It uses the public
REST gateway and therefore exercises authentication, risk checks, holds,
order routing, me-core matching, fill persistence, ledger posting, positions,
and market-data publication through the normal exchange path.

The service provisions deterministic users `u_bot_001` through `u_bot_050`.
The default is 30 active bots, ten passive levels per side, and one buy plus
one sell IOC attempt per open market each round. The configured passive depth is
bounded to eight through twelve levels per side.

Useful environment variables:

- `BOT_COUNT`: 20-50, default `30`.
- `BOT_FUND_USDC`: idempotent demo funding per bot, default `100000`.
- `BOT_BOOK_LEVELS`: passive levels per side, default `10`, bounded to `8..12`.
- `BOT_INTERVAL_MS`: delay between rounds, default `30000`.
- `BOT_MARKETS_PER_ROUND`: number of new markets seeded per round, default `8`,
  bounded to `1..16`. Quotes are retained after acceptance so the bot does not
  cancel and recreate the entire catalog every cycle.
- `BOT_ROUNDS`: finite rounds for a test run; `0` means continuous.
- `BOT_TICKERS`: optional comma-separated allowlist of open tickers.

The initial catalog is seeded from the authoritative `Contracts.xlsx` workbook by
`services/seeds/000004_contract_catalog.sql`. Sports rows are intentionally excluded
from the Sarvex catalog. Workbook template rows are represented by deterministic demo
instances so the API never returns unresolved `<K>`, `<candidate>`, or rolling-window
placeholders. The full non-sports catalog contains 49 binary contracts and 47 scalar
futures contracts.

For a bounded local run against an already running stack:

```bash
GW_REST_URL=http://127.0.0.1:18080 \
BOT_COUNT=20 \
BOT_ROUNDS=2 \
cargo run --manifest-path services/Cargo.toml -p trade-bots
```
