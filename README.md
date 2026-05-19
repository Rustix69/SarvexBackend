# SarvaEX — Build Plan

This is the complete engineering build plan for SarvaEX, a regulated event derivatives exchange offering binary options and scalar futures, USDC-settled.

**Strategy:** build a demo-grade exchange in **4 weeks** with **production-grade service boundaries**. Replace stub implementations with production ones post-funding without moving any boundary or breaking any interface.

---

## Document Map (read in order)

| # | File | What it covers |
|---|---|---|
| 1 | `01_foundations.md` | Principles, glossary, system architecture, 12-service map, tech stack, capacity targets, phase boundaries |
| 2 | `02_protobufs.md` | Every wire contract: gRPC service definitions, message types, REST OpenAPI, WebSocket protocol — **frozen on Day 1 of Week 1** |
| 3 | `03_databases.md` | Postgres schemas per service, account naming convention, ledger constraint triggers, seed data for 2 flagship contracts |
| 4 | `04_me_core.md` | Matching engine deep dive: Liquibook integration, single-thread sequencer, STP, journal/snapshot (production), demo recovery from Postgres |
| 5 | `05_services.md` | order-router, risk-svc, ledger-svc, position-svc, refdata-svc, audit-svc, admin-svc — designs, algorithms, failure modes |
| 6 | `06_oracle_settlement.md` | Oracle policies, settlement payout math (binary + scalar), idempotency, the demo settlement scene |
| 7 | `07_gateways.md` | gw-rest (REST/JSON) and gw-ws (WebSocket), auth, rate limiting, subscription protocol |
| 8 | `08_frontend.md` | Next.js 14 app, design tokens, real-time state management, all screens including admin |
| 9 | `09_demo_phase.md` | Week-by-week 4-week build plan, demo script, what to do when things break live |
| 10 | `10_production_phase.md` | Post-funding hardening roadmap, hiring plan, gate criteria |
| 11 | `11_runbook.md` | Operational runbook, demo reset, incident response, common operations |

---

## The Core Architectural Bet

The same architecture runs in the demo and in production. What changes is **what's behind each interface**:

| Component | Demo (Week 1–4) | Production (Month 5–14) |
|---|---|---|
| Matching engine | Liquibook + gRPC, no journal | + Journal, snapshot, HA standby, CPU pinning |
| Ledger | Real double-entry, "fake credit" deposits | + Fireblocks, wallet-watcher, reconciliation |
| Oracle | Admin button | Multi-source attestor + challenge window |
| Auth | JWT (HS256) | + MFA, API key signing, mTLS internal |
| Infra | docker-compose, one laptop | EKS in me-central-1, 3 AZs, full observability |
| Security | Plain gRPC, .env secrets | mTLS everywhere, KMS, pen-tested |

**No boundary moves. No protobuf changes. No data model migration.**

---

## What's Built in the Demo (Phase 0)

By end of Week 4:
- **12 services scaffolded:** gw-rest, gw-ws, order-router, risk-svc, ledger-svc, me-core (C++), position-svc, refdata-svc, oracle-svc, settlement-svc, audit-svc, admin-svc — plus mm-bots (Python) and web (Next.js)
- **Real matching:** Liquibook integrated, all matching real, single-thread sequencer per shard, STP enforced
- **Real ledger:** double-entry Postgres with balanced-transaction constraint trigger
- **Real settlement:** binary + scalar payout formulas; idempotent; with audit trail
- **Two flagship contracts seeded:** RBI June 2026 rate decision (binary), India CPI June 2026 (scalar)
- **MM bots** quoting both contracts realistically
- **Admin console** for the full lifecycle including oracle resolution
- **6-minute investor demo** runnable end-to-end with recorded backup

---

## What's Deliberately Not in the Demo

These are stubs/skipped because the investor value isn't worth the 4-week budget:
- Fireblocks / real on-chain custody (admin "fake credit" instead)
- Hot-standby me-core / journal+snapshot (single-process, rebuild from DB on restart)
- mTLS / KMS / API key signing
- Multi-source attestor oracle
- KYC vendor integration
- Mobile, multi-language, light theme
- Cross-margin
- FIX gateway

Each has a defined slot in the production phase (`10_production_phase.md`).

---

## Day 1 of Week 1: What Must Be Right

The single most important deliverable of Week 1 is **frozen protobufs** (`02_protobufs.md`). Once these are committed and tagged, the team can build all 13 services in parallel because the contracts are stable.

If you change a proto in Week 3, you potentially break Week 2's frontend work. Don't do this.

---

## How to Use This Plan

1. **Read the documents in order.** They build on each other.
2. **Day 1:** scaffold the repo per `01_foundations.md` §7.
3. **Day 2:** write every `.proto` per `02_protobufs.md` and tag `proto-v1.0.0`.
4. **Day 3:** apply migrations per `03_databases.md` and seed.
5. **Day 4–7:** build service skeletons per `05_services.md`, `06_oracle_settlement.md`, `07_gateways.md`.
6. **Week 2:** real matching path and frontend bones per `09_demo_phase.md` days 8–14.
7. **Week 3:** lifecycle, oracle, settlement, admin per `09_demo_phase.md` days 15–21.
8. **Week 4:** polish, rehearsal, lock per `09_demo_phase.md` days 22–28.

Then run the live demo. Update `ROADMAP.md` every other Friday during the raise.

After the seed closes, follow `10_production_phase.md` month by month.

---

## Living Document

Treat this as a **living plan**. As you learn things (investor questions, technical surprises, team feedback), update the relevant document. Tag versions in git so you can see how the plan evolved.

Suggested cadence:
- Bi-weekly review during fundraise: 30 minutes, walk through any changes.
- Monthly review during build: 1 hour, full architecture check-in.
- Every ADR (architecture decision record) added under `docs/adr/<NNNN>-<slug>.md`.

---

## Key Files for Investors / Technical Diligence

Hand a non-technical investor:
- The 6-minute demo
- The recorded backup
- A 1-page summary

Hand a technical investor doing diligence:
- This `README.md`
- `01_foundations.md`, `04_me_core.md`, `06_oracle_settlement.md`
- The architecture diagram from `01_foundations.md` §6
- The 4-week plan and the production roadmap

---

## Contact for Questions

Engineering questions go to the CTO.
Product / market questions go to the founder.
Compliance / regulatory questions go to the compliance officer (Phase 2A onward).

This document doesn't replace conversation. When you're stuck or unsure, talk to a human before guessing.
