# Sarvaex Agent Handoff

This file is a compact handoff from the previous Codex session. It records the
important product decisions, repository state, implementation history, deploy
workflow, and known constraints so another coding agent can continue safely.

Do not put passwords, private keys, JWT secrets, API keys, or bearer tokens in
this file. Use environment variables, the EC2 secret store, or the operator's
local SSH configuration.

## Current Repository State

- Repository: SarvexBackend / Sarvaex prediction-market platform.
- Local path used by the previous agent: /home/rustix/Quant Insider/sarvaex.
- Main branch: main.
- Current HEAD and origin/main: 664d4d2 Add trending discover page and trade confirmations.
- The latest frontend work is already committed and pushed to origin/main.
- The current worktree has one unrelated modified file: .gitignore.
- Do not stage .gitignore unless the user explicitly asks. It has an added test/
  ignore rule.
- Frontend source is under frontend/.
- Rust services are under services/.
- Frozen protobuf contracts are under proto/.
- C++ matching-engine source is under third_party/liquibook/ and the active
  C++ service boundary is under services/me-core/.
- frontend/dist/, frontend/node_modules/, services/target/, and local caches
  are generated artifacts and should not be committed.

Useful status commands:

    git status --short --branch
    git log --oneline --decorate -8
    git diff --check
    git remote -v

The GitHub remote is:

    git@github.com:Rustix69/SarvexBackend.git

## Product Context

Sarvaex is an MVP event/prediction-market exchange inspired by Kalshi. It has:

- Binary YES/NO markets.
- Numeric futures with long/short trading and candlestick charts.
- Live order books, fills, trade history, positions, balance, PnL, and orders.
- Public REST market data.
- Private REST account/trading APIs.
- WebSocket market/private event delivery.
- A C++ Liquibook matching-engine process behind a typed service boundary.
- Rust services for gateway, order routing, risk, ledger, positions, refdata,
  oracle, settlement, RFQ, audit, admin, market data, and bot simulation.

The frontend is intentionally a dense exchange/trading UI rather than a
marketing landing page. Keep controls compact, readable, and operational.

## Conversation/Product History

1. The graph was changed from a flat line to a more realistic zig-zag style.
   Binary charts use an interactive implied-probability chart with bid/ask
   band, fills, hover tooltip, latest percentage, and right-side percentage
   labels. Futures use candlestick charts through klinecharts/the futures chart
   implementation.
2. Sarvaex branding was standardized. Use SarvaEX/Sarvaex consistently as the
   product name, with the company logo beside the navbar brand. Do not
   reintroduce Kairos, Kalshi, or demo-brand text in the application.
3. Market cards received category-aware icons/images and a compact
   exchange-card layout. Sports sections were removed from active frontend views.
4. Futures received a separate navbar entry, terminal flow, numeric contracts,
   futures-specific payoff/margin UI, and candlestick charts. Binary and
   futures trading remain distinct.
5. The Portfolio page was built with account metrics, calendar/chart views,
   positions, orders, live PnL, realized/unrealized PnL, and a curve chart.
   Referrals and old portfolio subnavigation were removed. Position tickers now
   open the matching contract in Terminal.
6. The frontend received Merriweather, Geist Mono for numeric data, and
   Instrument Serif for the large hero headline where appropriate. Keep numeric
   order-book and trading values in Geist Mono.
7. Search was made functional. Search results select the contract and open its
   terminal view.
8. Health remains a route/page but was removed from the navbar. The login flow
   intentionally has no client-side timeout; do not add the old 15-second auth
   timeout back.
9. API documentation was added as a Trading API frontend section with endpoint
   navigation, request console, cURL/Python/JavaScript generation, WebSocket
   docs, search/command palette, auth controls, and separate scrolling sidebar,
   reference, and console panels. White code/URL fields must use black text.
10. Authentication was moved from demo-only identity to real account auth. The
    frontend supports account registration/login, session persistence across
    refreshes, profile/API-key management, API key copying, and logout.
11. The API supports Bearer JWT and X-API-Key credentials. API keys are scoped,
    stored hashed, shown in plaintext only at creation, and should be used for
    API trading/account operations.
12. Trade confirmations were added. Placing an order now shows a popup with
    contract, direction, order type, quantity, and status. Open/partial limit
    orders are watched by the existing refresh cycle and show a new popup when
    they transition to filled.
13. The Discover hero was changed from the old matrix slogan to a large
    featured event card. It shows the FOMC contract when available, settlement
    source, implied probability, YES/NO prices, and an interactive graph. The
    live-markets rail remains on the default landing view.
14. The Discover landing view is now Trending, not All. Trending is curated as
    up to four contracts per category, prioritizing live contracts, recent
    fills, and P1/P2/P3 launch priority. Category filters hide the large
    featured card and live rail and show only compact matching cards.

## Current Frontend Behavior

Main frontend files:

    frontend/src/App.jsx
    frontend/src/App.css
    frontend/src/index.css

Important routes:

    /                       Discover / Trending
    /futures                Futures catalog
    /portfolio              Portfolio
    /profile                Account/profile/API keys
    /trading-api            Trading API documentation
    /health                 Health page; intentionally not in navbar

The main navigation contains Discover, Futures, Terminal, Portfolio, and
Trading API. Health is intentionally omitted from the navbar.

Important implementation details in frontend/src/App.jsx:

- MarketDashboard defaults to Trending.
- buildTrendingMarkets() selects up to four per category.
- FOMC/federal-funds markets are preferred for the featured card.
- FeaturedMarketCard shows settlement source and MidPriceChart.
- TradeConfirmation displays immediate order response status.
- orderStatusesRef detects later open/partial -> filled transitions.
- PositionSnapshot is grouped directly below the binary/futures trade ticket.
- Portfolio position tickers call the existing handleMarketSelect() terminal
  navigation handler.

Frontend checks:

    npm run lint --prefix frontend
    npm run build --prefix frontend
    git diff --check

The latest frontend checks passed. Vite may print a non-fatal warning that the
main JavaScript chunk is larger than 500 kB.

## Backend Architecture

The active Rust workspace contains:

    services/
      crates/
      gw-rest/
      gw-ws/
      order-router/
      risk-svc/
      ledger-svc/
      position-svc/
      refdata-svc/
      oracle-svc/
      settlement-svc/
      audit-svc/
      admin-svc/
      marketdata-svc/
      me-core/
      me-core-adapter/
      rfq-svc/
      trade-bots/
      loadtest/
      migrations/
      seeds/
      docker-compose.yml

The Rust workspace uses Axum for public HTTP/WebSocket gateways, tonic/prost
for internal protobuf/gRPC, SQLx/PostgreSQL for durable state, and NATS for
event delivery. Liquibook remains inside the C++ me-core process. Rust must not
mutate matching state directly or move DB/NATS/ledger work into Liquibook
callbacks.

Important backend invariants:

- me-core is the single writer for each matching book.
- Every matched fill becomes an immutable Sarvaex fill fact.
- Order-router persists fills and outbox rows transactionally.
- Ledger is the balance authority; ledger operations are append-only,
  balanced, and idempotent.
- global_seq/contract_seq support fill replay and position gap recovery.
- A me-core timeout after enqueue is an unknown outcome, not an automatic
  rejection.
- Holds are released only after a terminal outcome is known.
- WebSocket snapshot and delta delivery must protect against snapshot races.
- Settlement waits for close sequence, fills, holds, ledger posting, and
  position consumers to catch up.

Read these before modifying backend behavior:

    services/README.md
    services/SECURITY_AND_OPERATIONS.md
    planning.md
    milestone_updates.md
    proto/sarvex/v1/
    services/docker-compose.yml

## Contracts and Data

The authoritative contract catalog came from the workbook previously named
Contracts.xlsx. The Rust seed is:

    services/seeds/000004_contract_catalog.sql

The catalog contains concrete binary and scalar-futures contracts. Do not
return unresolved <K>, <candidate>, or template values from the API. Known
resolved demo assumptions include:

    TASI above 11,500 on 29 Oct 2026?
    Nikkei above 42,000 on 30 Oct 2026?
    EU gas (TTF) above EUR 35 end-Oct?
    Josh Shapiro as the deterministic presidential nominee instance

The frontend merges the live API catalog with its static catalog and filters
sports out of active views. Preserve correct category mapping, especially
Crypto; do not fall back to Other for Bitcoin, BTC, Ethereum, or ETH contracts.

## Authentication and API Rules

Current production code path:

- AUTH_MODE=jwt uses HS256 JWTs.
- Required configuration includes JWT_SECRET (at least 32 bytes), issuer,
  audience, and TTL.
- Passwords are Argon2id hashes in PostgreSQL and never returned.
- API clients use X-API-Key: svx_live_<secret>.
- API key plaintext is returned only once on creation; DB stores hash/prefix,
  owner, scopes, expiry, and revocation timestamp.
- Trading mutations require trading:write.
- WebSocket connections require websocket:read.
- Local/demo mode (AUTH_MODE=demo) is only for local/demo operation and must
  not be exposed as production authentication.

Common public endpoints:

    GET  /v1/health/overview
    GET  /v1/health/live
    GET  /v1/health/ready
    GET  /v1/markets?state=OPEN&limit=200
    GET  /v1/markets/:ticker
    GET  /v1/markets/:ticker/orderbook?depth=12
    GET  /v1/markets/:ticker/fills
    GET  /v1/series
    GET  /v1/events

Private/trading endpoints include account balance/history/risk, orders, fills,
positions, demo deposits, API-key management, RFQ, and settlement read APIs.
Order mutations require an Idempotency-Key header and the body must contain a
positive count. Market orders may use the frontend market-price handling; if
the backend requires price_ticks, the client must send a positive resolved
price for the relevant market/order type.

## Docker and Local Runtime

The active compose file is:

    services/docker-compose.yml

Local stack:

    docker compose -f services/docker-compose.yml up --build

Shutdown command. This destroys local named volumes, so use only for a
development stack after confirming the target environment:

    docker compose -f services/docker-compose.yml down -v

The stack includes PostgreSQL, NATS/JetStream, migrations/seeds, refdata,
ledger, risk, me-core, me-core-adapter, marketdata, order-router, RFQ,
position, REST gateway, WebSocket gateway, oracle, settlement, and trade bots.

Important local host ports:

    PostgreSQL 15432
    NATS       14222 (monitor 18222)
    refdata    18061
    ledger     18062
    risk       18063
    me-core    15054
    order      18065
    position   18066
    marketdata 18067
    gw-rest    18080
    gw-ws      18082 (verify current compose if changed)
    trade bots 18090

Rust checks through services/Makefile:

    cd services
    make fmt
    make check
    make test
    make build
    make phase01-up
    make phase01-down

Do not run the full Docker stack on a low-memory laptop unless specifically
requested. Earlier work showed local non-root Docker access was denied, while
sudo docker ps worked; newgrp was unavailable. The intended heavy runtime/build
host is EC2.

## EC2 Deployment Workflow

No private EC2 host, username, or key is written here. The operator should
provide them through environment variables or local SSH config:

    export SARVAEX_EC2_HOST='user@ec2-host'
    export SARVAEX_EC2_KEY="$HOME/.ssh/sarvaex-ec2.pem"

Connect:

    ssh -i "$SARVAEX_EC2_KEY" "$SARVAEX_EC2_HOST"

If the local OpenSSH config has a permissions problem, use a direct config:

    ssh -F /dev/null -i "$SARVAEX_EC2_KEY" "$SARVAEX_EC2_HOST"

The previous Codex sandbox could not reliably do EC2 or GitHub network
operations because DNS/network sockets were restricted. A normal operator
terminal or CI runner is required for deployment.

On EC2, inspect before changing anything:

    cd /path/to/sarvaex
    git status --short --branch
    git pull --ff-only origin main
    docker ps
    docker compose -f services/docker-compose.yml ps

Deploy/rebuild the Rust stack:

    docker compose -f services/docker-compose.yml up -d --build
    docker compose -f services/docker-compose.yml ps

Check logs and public health:

    docker compose -f services/docker-compose.yml logs --tail=200 gw-rest
    docker compose -f services/docker-compose.yml logs --tail=200 order-router
    docker compose -f services/docker-compose.yml logs --tail=200 me-core
    docker compose -f services/docker-compose.yml logs --tail=200 trade-bots
    curl -fsS https://api.sarvaex.com/v1/health/overview

For frontend deployment, build from frontend/ and use the host's existing
web-server/hosting process. Do not invent a new hosting provider or overwrite
production files without checking the current service.

## Trade Bots and Market Depth

The Rust trade-bots service was added to make all contracts visibly active:

- Default deterministic demo bot count is 30, clamped to 20-50.
- Bots use the normal gateway/risk/hold/matching/fill/ledger path.
- Bots can seed passive bid/ask depth and generate crossing IOC fills.
- BOT_BOOK_LEVELS bounds passive depth; default is 1 to avoid exhausting
  collateral across the large catalog.
- The intent is many real Liquibook levels per side without unbounded order
  growth or fake frontend-only books.
- Bot configuration supports gateway URL, count, funding, interval, rounds,
  and ticker filters. Inspect services/trade-bots/README.md and compose
  environment before changing values.

When debugging a thin order book, first check bot health, active orders, risk
limits, balances/holds, and order-router/me-core logs. Do not solve liquidity
by fabricating order-book rows in the frontend.

## Common Verification Commands

Rust:

    cd services
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace
    cargo build --workspace

Frontend:

    npm run lint --prefix frontend
    npm run build --prefix frontend

Docker image example:

    docker build -f services/refdata-svc/Dockerfile .

Health/API smoke checks:

    curl -fsS http://127.0.0.1:18080/readyz
    curl -fsS 'http://127.0.0.1:18080/v1/markets?state=OPEN&limit=10'
    curl -fsS 'http://127.0.0.1:18080/v1/markets/<TICKER>/orderbook?depth=12'

Never paste live API keys or Bearer tokens into source, commits, logs, or this
handoff file. Use a shell variable:

    export SARVAEX_API_KEY='retrieve-from-secret-store'
    curl -H "X-API-Key: $SARVAEX_API_KEY" https://api.sarvaex.com/v1/account/balance

## Known Gaps / Deferred Work

Confirm the current code before claiming these are production-complete:

- Full cross-language C++ me-core gRPC integration and integration tests.
- Production-grade me-core WAL/snapshot guarantees beyond the demo journal.
- Live orderbook snapshot/delta race testing under restart.
- Full production event-retention/JetStream inspection.
- Ledger fill-outbox and settlement-consumer end-to-end tests.
- Oracle attestation and settlement production hardening.
- External OIDC/RS256 auth, password reset, email verification, and MFA.
- Production rate limits, observability dashboards, load tests, and failover.
- EC2 deployment verification after each new backend change.

The current frontend changes are already in 664d4d2, but always verify the
actual remote and EC2 deployment state before telling the user production is
updated.

## Agent Operating Rules

- Read existing code and local patterns before editing.
- Keep frontend changes in frontend/; keep backend changes in their service.
- Do not change frontend source while doing backend-only work.
- Do not delete databases, Docker volumes, order fills, or production data
  without explicit confirmation for that exact environment.
- Do not commit credentials, .pem files, API keys, tokens, or .env files.
- Preserve unrelated user changes; stage specific files only.
- Run focused tests plus the relevant build after edits.
- Report deployment/network blockers honestly; never claim a push or deployment
  that was not verified.
- For production pushes, inspect git status, stage only intended files, commit
  with a focused message, push origin main, then run EC2 health and smoke checks.

