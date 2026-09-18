# Sarvex — Complete Design & Build Plan

**Audience:** Engineering team
**Status:** Authoritative build spec
**Strategy:** Build a demo-grade exchange in 4 weeks with **production-grade service boundaries**. Replace stub implementations with production ones post-funding without moving any boundary or breaking any interface.

---

## 0. How to read this document

This is one document split across multiple files for readability:

- `01_foundations.md` — principles, glossary, architecture, service map, repo layout *(this file)*
- `02_protobufs.md` — every service contract, frozen on day 1
- `03_databases.md` — Postgres schemas for every service
- `04_me_core.md` — matching engine deep dive (Liquibook integration)
- `05_services.md` — order router, risk, ledger, position keeper, refdata
- `06_oracle_settlement.md` — oracle worker, settlement engine
- `07_gateways.md` — REST gateway, WSS gateway, auth
- `08_frontend.md` — Next.js app, screens, real-time data, state
- `09_demo_phase.md` — Week-by-week 4-week build plan
- `10_production_phase.md` — Post-funding hardening roadmap
- `11_runbook.md` — Operational runbook, demo script, reset procedures

Build in the order listed.

---

## 1. North Star

Sarvex is a regulated event derivatives exchange offering binary options and scalar event futures, denominated in USDC. The **demo build** proves the team can ship real exchange infrastructure. The **production build** is what we operate after raising seed capital.

**The non-negotiable principle:** every service boundary, protobuf, and event subject in this design is **production-grade from day 1**. Stubs and simplifications live behind these boundaries; the boundaries themselves never move.

This means:
- A demo `Risk Service` and a production `Risk Service` expose the same gRPC interface.
- A demo `Ledger Service` and a production `Ledger Service` write to the same Postgres schema.
- A demo `me-core` and a production `me-core` speak the same matching protocol.
- Replacing demo → production is a per-service substitution, never an architectural change.

---

## 2. Design Principles

1. **Real boundaries, stub implementations.** Interfaces are production-grade; what's behind them can be 200 lines for the demo.
2. **Journal first, mutate second.** Production-grade `me-core` writes to a durable journal before in-memory state mutates. Demo `me-core` rebuilds state from Postgres on restart; the journaling layer plugs in later without changing the gRPC interface.
3. **Single-writer for the book.** Matching is single-threaded per contract. Concurrency comes from sharding contracts.
4. **Idempotency at every external boundary.** `client_order_id`, deposit `tx_hash`, withdrawal request, settlement payouts — all keyed on idempotency tokens.
5. **Double-entry for all money movement.** No balance changes except via a balanced ledger transaction.
6. **Sequence numbers everywhere.** Market data, ledger entries, audit events — each carries a monotonic seq within its scope.
7. **Service owns its data.** No service reads another service's tables directly. Cross-service data access is via gRPC or NATS. Any demo-only exception (for example `me-core` boot restore from `orders.orders`) must be explicitly listed in the schema ownership map and removed in production.
8. **Stateless services scale horizontally; stateful services own one database.**
9. **All time is UTC, all monetary values are integers** (USDC: `micro_usdc` = 1e-6 USDC). No floats anywhere in the money path.

---

## 3. Glossary

| Term | Definition |
|---|---|
| **Contract** | A tradable instrument. Binary or scalar. |
| **Binary contract** | YES/NO option. Settles $1 to winners, $0 to losers. Priced in cents (1–99). |
| **Scalar contract** | Numeric-settled future. Payout = `clamp((realized − lower)/(upper − lower), 0, 1) × multiplier`. Priced in ticks (e.g., basis points). |
| **Event** | The real-world outcome (e.g., "RBI June 2026 rate decision"). One event may back multiple contracts. |
| **Series** | A family of related events (e.g., "RBI rate decisions, all meetings"). |
| **Ticker** | Unique contract identifier. Format: `<SERIES>-<EVENT>-<CONTRACT>` e.g., `RBI-JUN26-CUT25`. |
| **Tick** | Minimum price increment. Binary: 1 cent. Scalar: contract-defined. |
| **Hold** | Reserved (uncommitted) USDC against an open order. |
| **Position** | Net contract count per user per contract, signed. |
| **Order ID** | 64-bit Snowflake assigned by Order Router. Globally unique. |
| **Sequencer** | Single-threaded component in `me-core` that imposes total order on commands. |
| **Journal** | Append-only durable log of commands. (Production only; demo: not implemented.) |
| **Snapshot** | Serialized `me-core` state at a known journal offset. (Production only.) |
| **Liquibook** | Header-only C++ template order-book/matching library used inside `me-core`. |
| **me-core** | The matching engine service. Wraps Liquibook + adds gRPC + (production) journal. |
| **MD** | Market Data. |
| **Resolution** | Oracle-driven determination of an event outcome. |
| **Settlement** | Ledger-side payout to position holders after resolution. |
| **STP** | Self-Trade Prevention. |
| **TIF** | Time-in-Force: `GTC`, `IOC`, `FOK`. |
| **micro_usdc** | Internal monetary unit. 1 USDC = 1,000,000 micro_usdc. All money in code is int64. |

---

## 4. System Context

```
                           Sarvex Trust Boundary
                  ┌───────────────────────────────────────┐
                  │                                       │
  Retail trader ──┤                                       │
                  │           Sarvex services            ├──► [Demo] Mocked
  Institutional ──┤             (12 services)             ├──► [Prod] Fireblocks (USDC custody)
  trader / LP     │                                       │
                  │                                       ├──► [Demo] JSON file
  Admin / Ops ────┤                                       ├──► [Prod] Oracle attestors / chain RPCs
                  │                                       │
                  └───────────────────────────────────────┘
                                   ▲
                                   │
                       Audit / regulator (Phase 2)
```

**External integrations:**

| System | Demo state | Production state |
|---|---|---|
| USDC custody | Stubbed (admin "fake credit") | Fireblocks MPC, hot/warm/cold wallets |
| Chain deposit detection | None | wallet-watcher on Alchemy/QuickNode |
| Withdrawal signing | None | Fireblocks policy engine |
| Oracle resolution | JSON file + admin button | K-of-N attestor signatures, challenge window |
| KYC | Hardcoded users | Sumsub or Persona |
| Edge | docker-compose port mapping | CloudFront + AWS WAF |
| Observability | stdout logs | Prometheus + Tempo + Loki + PagerDuty |
| Secrets | `.env` files | AWS Secrets Manager + KMS |

---

## 5. Service Map

Twelve services. Every service exists from day 1. Every service has a real protobuf, a real database schema (if stateful), and real boundaries.

| # | Service | Language | State | Public? | Demo LOC est. | Prod LOC est. |
|---|---|---|---|---|---|---|
| 1 | `gw-rest` | Go | Stateless | Yes (HTTPS) | 600 | 3,500 |
| 2 | `gw-ws` | Go | Stateless | Yes (WSS) | 500 | 2,500 |
| 3 | `order-router` | Go | Postgres (orders) | No | 700 | 2,800 |
| 4 | `risk-svc` | Go | Postgres (limits) | No | 400 | 2,000 |
| 5 | `ledger-svc` | Go | Postgres (accounts, entries) | No | 700 | 4,000 |
| 6 | `me-core` | C++ (Liquibook) | In-memory (+ journal in prod) | No | 1,500 | 6,000 |
| 7 | `position-svc` | Go | Postgres (positions) | No | 300 | 1,500 |
| 8 | `refdata-svc` | Go | Postgres (contracts) | No | 400 | 2,000 |
| 9 | `oracle-svc` | Go (Rust in prod) | Postgres (attestations) | No | 400 | 3,500 |
| 10 | `settlement-svc` | Go | Postgres (settlements) | No | 400 | 2,000 |
| 11 | `audit-svc` | Go | Postgres (events) | No | 200 | 1,500 |
| 12 | `admin-svc` | Go | Stateless | No (VPN only in prod) | 400 | 2,500 |
| — | `mm-bots` | Python | Stateless | No | 500 | 1,500 |
| — | `web` | TypeScript / Next.js | Stateless | Yes | 4,000 | 25,000 |

**Total demo:** ~11,000 LOC across services + 4,000 LOC frontend.
**Total production:** ~60,000 LOC across services + 25,000 LOC frontend.

The demo is ~18% of the production codebase. The architecture is 100%.

---

## 6. High-Level Architecture

```
┌────────────────────────────────────────────────────────────────────────┐
│                       CLIENT EDGE                                       │
│                                                                         │
│        Browser (Next.js)        Admin Console (Next.js)                │
│           │  │                       │                                  │
│           │  └─ WSS ───────────┐     │                                  │
│           │                    │     │                                  │
│           └─ REST/HTTPS ───┐   │     │ REST                             │
│                            ▼   ▼     ▼                                  │
│                       ┌──────────┐ ┌──────────┐ ┌──────────┐           │
│                       │ gw-rest  │ │ gw-ws    │ │admin-svc │           │
│                       └─────┬────┘ └────┬─────┘ └────┬─────┘           │
└─────────────────────────────┼───────────┼────────────┼─────────────────┘
                              │           │            │
                       gRPC   │     NATS  │      gRPC  │
                              ▼           │            ▼
   ┌──────────────────────────────────────┼─────────────────────────┐
   │                                      │                          │
   │  ┌────────────────┐                  │  ┌──────────────────┐    │
   │  │ order-router   │◄─────────────────┼──┤ risk-svc         │    │
   │  └────┬────┬──────┘                  │  └──────────────────┘    │
   │       │    │                         │                          │
   │  gRPC │    │ gRPC                    │                          │
   │       ▼    ▼                         │  ┌──────────────────┐    │
   │  ┌─────────┐  ┌────────────┐         │  │ refdata-svc      │    │
   │  │ledger   │  │ me-core    │─────────┤  └──────────────────┘    │
   │  │-svc     │  │ (Liquibook)│         │                          │
   │  └─────────┘  └─────┬──────┘         │  ┌──────────────────┐    │
   │                     │                │  │ position-svc     │◄───┤
   │                     │ publish        │  └──────────────────┘    │
   │                     ▼                │                          │
   │           ┌─────────────────────────────────────────────┐       │
   │           │           NATS (event spine)                │       │
   │           │   md.book.*    md.trade.*                   │       │
   │           │   ledger.events oracle.attestations         │       │
   │           │   exec.events exec.fills.* audit.events                       │       │
   │           └─────────────────────────────────────────────┘       │
   │                     ▲                                            │
   │                     │                                            │
   │  ┌─────────────┐    │   ┌────────────────┐  ┌──────────────┐   │
   │  │ oracle-svc  │────┘   │ settlement-svc │  │ audit-svc    │   │
   │  └─────────────┘        └────────────────┘  └──────────────┘   │
   │                                                                  │
   │  ┌─────────────────────────────────────────────────────────┐    │
   │  │           Postgres (per-service schemas)                 │    │
   │  │  orders | risk | ledger | refdata | position |          │    │
   │  │  oracle | settlement | audit                            │    │
   │  └─────────────────────────────────────────────────────────┘    │
   │                                                                  │
   │  ┌──────────────┐                                                │
   │  │ Redis        │  (idempotency cache, rate limits)              │
   │  └──────────────┘                                                │
   └──────────────────────────────────────────────────────────────────┘

         ┌─────────────┐
         │ mm-bots     │  (Python, talks to gw-rest + gw-ws like a real client)
         └─────────────┘
```

**Communication patterns:**

- **gRPC (HTTP/2 + protobuf)** for synchronous service-to-service in the hot path.
- **NATS** for the event spine (market data, execution, ledger, audit, oracle).
  - Demo: NATS Core (in-memory, lossy under crash). Demo durability comes from Postgres rows/outboxes and me-core in-memory state, not from NATS.
  - Production: NATS JetStream (durable, replicated R3) plus me-core journal for replay/failover.
- **WebSocket** for client streaming, JSON.
- **REST/HTTPS** for client order entry, account, history.

**Canonical event subjects:**

| Subject | Payload | Producer | Consumers |
|---|---|---|---|
| `md.book.<ticker>` | `OrderBookDeltaEvent` | me-core | gw-ws |
| `md.trade.<ticker>` | `TradeEvent` public tape | me-core | gw-ws, frontend |
| `md.ticker.<ticker>` | `TickerEvent` | me-core | gw-ws |
| `exec.events` | `ExecutionEvent` internal stream | me-core | order-router, risk-svc |
| `exec.fills.<ticker>` | fill-only `ExecutionEvent` | me-core/order-router publisher | position-svc, fill-poster, risk-svc |
| `exec.user.<user_id>` | sanitized user order event | order-router | gw-ws |
| `exec.fills.user.<user_id>` | sanitized user fill event | order-router | gw-ws |
| `ledger.events` / `ledger.balance.user.<user_id>` | ledger event / balance update | ledger-svc | audit-svc, gw-ws |
| `oracle.resolutions.finalized.<event_ticker>` | finalized resolution | oracle-svc | settlement-svc |
| `audit.events` | `AuditEvent` | all services | audit-svc |

---

## 7. Repository Layout

Single monorepo. One developer can `git clone` and `./scripts/run-demo.sh` to bring up everything.

```
sarvex/
├── README.md
├── ROADMAP.md                      # Living document, updated bi-weekly
├── docker-compose.yml              # Demo deployment
├── .env.example
├── Makefile                        # Common targets: build, test, run, reset
│
├── proto/                          # FROZEN on Day 1 of Week 1
│   ├── sarvex/v1/common.proto
│   ├── sarvex/v1/order.proto
│   ├── sarvex/v1/match.proto
│   ├── sarvex/v1/ledger.proto
│   ├── sarvex/v1/risk.proto
│   ├── sarvex/v1/refdata.proto
│   ├── sarvex/v1/oracle.proto
│   ├── sarvex/v1/settlement.proto
│   ├── sarvex/v1/marketdata.proto
│   └── sarvex/v1/audit.proto
│
├── services/
│   ├── gw-rest/             # Go
│   ├── gw-ws/               # Go
│   ├── order-router/        # Go
│   ├── risk-svc/            # Go
│   ├── ledger-svc/          # Go
│   ├── me-core/             # C++, Liquibook submodule
│   │   ├── CMakeLists.txt
│   │   ├── src/
│   │   └── third_party/liquibook/  # git submodule
│   ├── position-svc/        # Go
│   ├── refdata-svc/         # Go
│   ├── oracle-svc/          # Go (Rust in prod)
│   ├── settlement-svc/      # Go
│   ├── audit-svc/           # Go
│   ├── admin-svc/           # Go
│   └── mm-bots/             # Python
│
├── web/                            # Next.js 14, App Router
│   ├── app/
│   ├── components/
│   ├── lib/
│   └── package.json
│
├── db/
│   ├── migrations/                 # One subdir per service schema
│   │   ├── orders/
│   │   ├── ledger/
│   │   ├── ... (one per service)
│   └── seed/
│       ├── contracts.sql
│       └── demo_users.sql
│
├── scripts/
│   ├── run-demo.sh                 # docker-compose up + seed + open browser
│   ├── reset-demo.sh               # Reset to clean state in ~20s
│   ├── seed-demo-data.sh
│   ├── proto-gen.sh                # Regenerate language bindings from proto/
│   └── record-demo.sh
│
├── pkg/                            # Shared Go libraries
│   ├── auth/
│   ├── idempotency/
│   ├── tracing/
│   ├── pgx/                        # Postgres helpers
│   └── natsutil/
│
└── docs/
    ├── 01_foundations.md           # This file
    ├── 02_protobufs.md
    ├── 03_databases.md
    ├── 04_me_core.md
    ├── 05_services.md
    ├── 06_oracle_settlement.md
    ├── 07_gateways.md
    ├── 08_frontend.md
    ├── 09_demo_phase.md
    ├── 10_production_phase.md
    └── 11_runbook.md
```

---

## 8. Technology Choices (Demo + Production)

| Concern | Demo | Production | Rationale |
|---|---|---|---|
| Service language | Go | Go | One language for all non-ME services. Boring, productive, great gRPC support. |
| ME language | C++ | C++ | Liquibook is C++. No translation layer. |
| Frontend | Next.js 14 | Next.js 14 | App Router, real-time WebSocket, edge-deployable later. |
| RPC | gRPC (plaintext) | gRPC (mTLS) | Same wire format; add TLS in production phase. |
| Event bus | NATS Core | NATS JetStream | Same client API; JetStream is just NATS + durability. |
| Database | Postgres 16 | Postgres 16 (Multi-AZ + PITR) | Same schemas; HA layer added in production. |
| Cache | Redis 7 | Redis 7 (Sentinel) | Same. |
| Auth | JWT (HS256) | JWT (RS256) + Ed25519 API keys + MFA | JWT stays; signing/MFA layers added. |
| Deployment | docker-compose | Kubernetes (EKS) on AWS | Containers are same; orchestration changes. |
| Region | localhost | `me-central-1` (Bahrain) | DIFC-aligned. |
| Custody | Stub | Fireblocks MPC | Stub uses same `Ledger` interface; swap occurs in `ledger-svc` internals. |
| Oracle | JSON file + admin button | K-of-N attestor + challenge window | Same `oracle.attestations` topic and protobuf. |
| Observability | stdout JSON logs | Prometheus + Tempo + Loki | Add scrape endpoints + tracing SDK; code instrumentation stays. |

The key insight: **every demo choice is on the path to its production counterpart**. Nothing gets thrown away.

---

## 9. Capacity Targets

| Metric | Demo target | Production target |
|---|---|---|
| Orders/sec sustained | 100 | 10,000 |
| Orders/sec burst | 500 | 30,000 |
| Concurrent users | 10 | 100,000 |
| Concurrent WSS connections | 20 | 300,000 |
| Active contracts | 5 | 500 |
| Order ack p99 | 100 ms | 25 ms |
| ME match latency p99 | n/a (no profiling) | 1.6 ms |
| ME failover RTO | n/a (cold restart) | 5 s |
| Settlement complete after resolution | <30s | <60s |
| Reconciliation drift | n/a (no chain) | $0 |

Demo capacity is sized for live walkthrough: 1–3 users actively trading, MM bots quoting, hundreds of orders per minute. Production is sized for institutional event derivatives at scale.

---

## 10. Phase Boundaries (Demo → Production)

### Phase 0: Demo Build (Weeks 1–4)
All 12 services scaffolded with real interfaces. Stubbed implementations where production-grade isn't required for the investor narrative.

### Phase 1: Fundraise Polish (Weeks 5–16, variable)
Iterative improvements to the demo based on investor feedback. Capped at ~10 eng-hours/week.

### Phase 2: Post-Funding Hardening (Months 5–10)
Replace stubs with production implementations, service by service. No boundaries move.

### Phase 3: Launch Readiness (Months 11–14)
Compliance, beta, soak testing, runbooks, DFSA pre-launch dialogue.

Full week-by-week plans are in `09_demo_phase.md` and `10_production_phase.md`.

---

## 11. What This Document Is Not

- **Not a sales document.** It's an engineering build spec.
- **Not a regulatory submission.** DFSA-grade documentation comes in Phase 3.
- **Not a cross-margin design.** Phase 1 is single-contract margin only.
- **Not on-chain.** USDC is treated as a fiat-style book-entry currency.
- **Not mobile.** Web only through Phase 3.

Anything in those buckets is explicitly out of scope until later phases.
