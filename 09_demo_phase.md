# 09 — Demo Build Plan (Phase 0, Weeks 1–4)

Four weeks to a live, end-to-end investor demo. **All 12 services exist from Day 1** with real protobuf interfaces. Stub implementations behind real boundaries.

This plan assumes 1 strong full-stack engineer plus you. Add a second engineer and you shave a week or get more polish.

---

## Week 1 — Foundation + interfaces frozen

**Goal at end of week:** repo scaffolded, all protobufs written and approved, services compile and run as stubs, an order can be submitted → me-core → and the response returns. No real fills yet. No frontend yet.

### Day 1 (Monday) — Repo skeleton

- [ ] Create monorepo from §7 layout in `01_foundations.md`
- [ ] `docker-compose.yml` with: postgres, nats, redis (no app services yet)
- [ ] `Makefile` with targets: `make build`, `make test`, `make run`, `make reset`
- [ ] `scripts/proto-gen.sh` (uses `buf` for proto management)
- [ ] CI pipeline: GitHub Actions running `make build` + `make test` on every PR
- [ ] Commit `pkg/` skeletons (auth, idem, tracing, pgx, natsutil, snowflake, audit)

### Day 2 (Tuesday) — **PROTOBUFS FROZEN** (highest leverage day)

- [ ] Write every `.proto` from §2 in full
- [ ] `./scripts/proto-gen.sh` produces Go + C++ bindings cleanly
- [ ] Review meeting: walk through every proto with you; lock in
- [ ] Commit tag `proto-v1.0.0` — any future change requires explicit RFC

### Day 3 — Database migrations

- [ ] `db/migrations/` for every service schema from §3
- [ ] `golang-migrate` integrated into Makefile
- [ ] `make db-up` brings up Postgres + applies all migrations
- [ ] `db/seed/contracts.sql` with the 2 flagship contracts
- [ ] `db/seed/demo_users.sql` with 4 demo accounts
- [ ] Verify ledger balance trigger fires correctly with a hand-written balanced/unbalanced tx test

### Day 4 — Go service skeletons

For each of: `gw-rest`, `gw-ws`, `order-router`, `risk-svc`, `ledger-svc`, `position-svc`, `refdata-svc`, `oracle-svc`, `settlement-svc`, `audit-svc`, `admin-svc`:
- [ ] `cmd/server/main.go` boots, listens on gRPC port, registers health endpoint
- [ ] `internal/server/` has a service implementation with all methods returning `Unimplemented`
- [ ] Dockerfile builds
- [ ] `docker-compose.yml` runs the service

### Day 5 — me-core skeleton

- [ ] `services/me-core/` with CMake project
- [ ] Vendor Liquibook as git submodule under `third_party/liquibook`
- [ ] `src/main.cpp` boots, listens on gRPC port 50051, registers health
- [ ] `SubmitOrder` returns a fake accept ack
- [ ] Add to `docker-compose.yml`
- [ ] **End-of-week milestone:** `make run` brings up all 13 containers; `curl gw-rest:8080/health/live` returns 200 for every service.

### Day 6 — End-to-end "hello order" trace

- [ ] `gw-rest` implements `POST /v1/orders` returning a hardcoded ack
- [ ] `order-router.SubmitOrder` returns a hardcoded ack
- [ ] `me-core.SubmitOrder` (still no real matching) returns a hardcoded ack
- [ ] Run: `curl -X POST localhost:8080/v1/orders -d '{"ticker":"RBI-JUN26-CUT25","side":"yes","action":"buy","price_ticks":62,"count":10}'`
- [ ] Verify response. Take a screenshot. This is the smoke test.

### Day 7 — Buffer

Cleanup, document what you have so far. CI green. Tag `week1-complete`.

---

## Week 2 — Real trading path + frontend bones

**Goal at end of week:** an order can be placed via the frontend, hits a real Liquibook book, returns a fill or rests on book, appears in the order book displayed in the browser. MM bots quoting.

### Day 8 — me-core real matching

- [ ] `SarvaOrder` class
- [ ] `ShardState` with `DepthOrderBook<SarvaOrder*>`
- [ ] `SarvaListener` implementing OrderListener + TradeListener + DepthListener
- [ ] `Sequencer` thread + inbound MPSC ring
- [ ] `apply_submit`, `apply_cancel` paths
- [ ] Unit test: 3 limit buys, 2 limit sells, verify resulting fills + book state

### Day 9 — me-core integration

- [ ] STP enforcement in listener
- [ ] TIF (IOC, FOK, GTC) handling
- [ ] POST_ONLY rejection
- [ ] NATS publisher thread; emits `md.book.<ticker>`, `md.trade.<ticker>`, `exec.fills`
- [ ] gRPC `GetBookSnapshot` returns real book state
- [ ] Demo recovery: read OPEN/PARTIAL orders from `orders.orders` on startup; rebuild books
- [ ] Boot integration test: 10 orders submitted via gRPC → expected fills + book state

### Day 10 — Order router + risk + ledger MVP

- [ ] `order-router.SubmitOrder` does the full orchestration from §1.2 of `05_services.md`
- [ ] Snowflake ID generator
- [ ] Idempotency layer 1 (in-process map) and layer 2 (`UNIQUE(user_id, client_order_id)`)
- [ ] `risk-svc.PreTradeCheck` with sanity + balance approximation (real margin check in Week 3)
- [ ] `ledger-svc.PlaceHold` + `CommitHold` + `ReleaseHold` working with balanced transactions
- [ ] Integration test: place hold, submit order, fill it, verify ledger entries balanced

### Day 11 — gw-rest + gw-ws

- [ ] gw-rest: full middleware stack from §1.3 of `07_gateways.md`
- [ ] gw-rest: real `POST /v1/orders`, `GET /v1/markets/{ticker}/orderbook`, `GET /v1/account/balance`, `POST /v1/auth/login`
- [ ] gw-ws: WebSocket handshake, subscribe protocol, NATS bridge
- [ ] gw-ws: `orderbook_snapshot`, `orderbook_delta`, `trades` channels working
- [ ] Smoke test: open `wscat`, subscribe, place order via curl, see deltas flow

### Day 12 — Frontend bones

- [ ] Next.js 14 app scaffold; Tailwind + shadcn/ui set up
- [ ] Design tokens from §3.1 of `08_frontend.md`
- [ ] `/login` page; auth flow working end-to-end
- [ ] `/markets` page lists the 2 flagship contracts (via `GET /v1/markets`)
- [ ] `/markets/[ticker]` page with order book component subscribing to WS
- [ ] Order ticket: place market and limit orders
- [ ] Trade tape

### Day 13 — MM bots

- [ ] `services/mm-bots/` Python module
- [ ] One bot per contract; quotes both sides at configured spread around a midpoint
- [ ] Uses gw-rest (HTTPS) and gw-ws (WSS) — i.e., a real client, not a backdoor
- [ ] Each bot has a separate user account (`u_mm_1`, etc.) with seeded balance
- [ ] Bot updates quotes every 500ms with small random walks
- [ ] Run: bots fill the book; book looks "alive" in the frontend

### Day 14 — Polish + buffer

- [ ] Position panel works (subscribes to `user_fills`, computes display position from local trades since the API surface for positions isn't ready yet — finish in Week 3)
- [ ] Fix UI bugs surfaced by playing with the demo for an hour
- [ ] **End-of-week milestone:** screen-record a 90-second clip: login → markets list → click into a contract → see live book → place order → see fill → see position update. **Send to yourself for review.**

---

## Week 3 — Lifecycle + admin + settlement

**Goal at end of week:** the full lifecycle works. A contract opens, trades, closes, oracle resolves, settlement pays out. Admin console fully operational.

### Day 15 — position-svc + refdata-svc

- [ ] `position-svc` consumes `exec.fills`, maintains positions
- [ ] gw-rest hooks `/v1/positions` and `/v1/portfolio` to position-svc
- [ ] Frontend portfolio screen displays real positions
- [ ] `refdata-svc` lifecycle FSM with permitted transitions
- [ ] Background scheduler transitions LISTED → OPEN → CLOSED based on times
- [ ] `md.lifecycle.<ticker>` events published

### Day 16 — Oracle + settlement

- [ ] `oracle-svc.AdminForceResolution` — admin issues outcome with audit trail
- [ ] Publishes `oracle.resolutions.finalized.<event_ticker>`
- [ ] `settlement-svc` consumes, computes payouts for binary and scalar contracts
- [ ] Posts ledger transactions for each winner; sweeps rounding to REVENUE
- [ ] Transitions contract: CLOSED → RESOLVING → SETTLED
- [ ] Integration test: trade 100 contracts, resolve, verify all positions settled correctly

### Day 17 — Admin frontend

- [ ] `/admin` layout with role gate
- [ ] `/admin/users` — list users, freeze, credit fake deposit
- [ ] `/admin/contracts` — list contracts, transition state
- [ ] `/admin/oracle` — for each closing contract, "Resolve" action with outcome picker
- [ ] `/admin/audit` — live tail of audit events (uses WS channel `admin_audit_tail`)

### Day 18 — Audit + reset script

- [ ] `audit-svc` consumes `audit.events`, writes to `audit.events` table
- [ ] All services emit audit events via `pkg/audit/emitter.go`
- [ ] `scripts/reset-demo.sh`:
  - kills me-core
  - truncates orders, fills, positions, holds, ledger entries (except house seed), oracle resolutions, settlements, audit events
  - re-seeds users + contracts via `seed-demo-data.sh`
  - restarts services
  - target: completes in ≤ 30 seconds
- [ ] `scripts/seed-demo-data.sh` credits each demo user $1,000 via `Ledger.AdminCreditDeposit`

### Day 19 — Demo polish layer 1

- [ ] Marketing landing page (`/`) — short, confident, not generic
- [ ] Branding pass: logo, favicon, social card
- [ ] Empty states and loading skeletons
- [ ] Toast notifications for order events
- [ ] Real "Latency" indicator in header

### Day 20 — End-to-end demo dry run (internal)

Run the demo top to bottom, time it, fix glitches:
- [ ] Login as retail
- [ ] Browse markets, click into binary contract
- [ ] See live order book, MM quoting
- [ ] Place a market buy, see fill
- [ ] Switch to admin, advance contract clock, close trading
- [ ] Trigger oracle resolution
- [ ] Switch back to retail, see settlement
- [ ] Show audit log entries for the lifecycle

Total: should be 5-7 minutes when polished. Identify all rough edges.

### Day 21 — Buffer + bug fixes

Fix everything found in Day 20 dry run. Tag `week3-complete`.

---

## Week 4 — Polish + investor rehearsals

**Goal at end of week:** the demo is investor-ready, with a tight script, recorded backup, and 2-3 friendly viewers having seen it.

### Day 22 — Demo data polish

- [ ] Improve demo contracts: more realistic questions, real-ish prices
- [ ] Pre-seed some trade history so the chart isn't empty on first view
- [ ] Make MM bots quote slightly tighter spreads (looks more competitive)
- [ ] Make order book deeper (better visual signal)

### Day 23 — Visual polish

- [ ] Price chart with light history
- [ ] Subtle animations on book updates (flash + fade)
- [ ] State transition animations (contract state badge pulse on change)
- [ ] Settlement screen with subtle confetti (only during the demo settlement scene)
- [ ] About-this-demo modal explaining what's real vs stubbed

### Day 24 — Recording + backup plan

- [ ] Record full 6-minute walkthrough using OBS — this is the backup if live demo fails
- [ ] Test the recording in a clean Chrome incognito to verify everything works
- [ ] Save as both `.mp4` and `.webm`; upload to S3 + local copy
- [ ] Print the demo script as a one-pager you can have nearby

### Day 25 — Friendly demo #1

- [ ] Demo to one trusted technical person outside the team
- [ ] Capture every question they ask; note where they got confused
- [ ] Don't fix everything they say; fix the things that are clearly right

### Day 26 — Iteration

- [ ] Address Day 25 feedback
- [ ] Tighten the script wording
- [ ] If the friendly demo surfaced a missing feature, decide: fix this week or note for Phase 1

### Day 27 — Friendly demo #2 + final polish

- [ ] Demo to second trusted person
- [ ] Address remaining feedback
- [ ] Final visual pass: spacing, colors, typography
- [ ] Make sure reset script works perfectly (you'll run it between investor demos)

### Day 28 — Lock + handoff to investor calls

- [ ] **Stop making changes 2 days before your first investor demo.**
- [ ] Tag `demo-v1.0.0`
- [ ] Verify build is reproducible from clean clone: `git clone && make run && make seed && open http://localhost:3000` works in < 3 minutes on a fresh laptop
- [ ] Write a one-pager: "What's in the demo, what's stubbed, what's the path to production" — keep this for technical investor follow-ups

---

## Demo script (6 minutes)

This is what you actually say during the live demo.

### Minute 0–1: Setup
> "SarvaEX is a regulated event derivatives exchange. We're building an institutional-grade venue for binary and scalar futures on real-world events — central bank decisions, inflation prints, election outcomes — settled in USDC.
> 
> Today I'll show you a working version. This isn't a mockup; every transaction you see is going through a real matching engine, a real double-entry ledger, and producing a real audit trail.
> 
> Two contracts seeded today: a binary on RBI's June rate decision, and a scalar future on India CPI June print."

### Minute 1–3: Trading
> [Click into RBI binary]
> 
> "This is the trading screen. Order book on the right, price history on the left. Market makers are quoting both sides; I'm seeing 200 lots at 62 cents on the bid, 200 at 64 on the ask.
> 
> Let me place an order. I want 100 contracts YES at 63 cents — that's a $63 max loss for $100 max payout."
> 
> [Place limit buy 100 @ 63]
> 
> "Order's resting on the book. You can see it on the bid side. Latency on the order ack was 23 milliseconds end-to-end."
> 
> [MM bot lifts the offer]
> 
> "And now a market maker took my bid. I'm long 100 YES at 63."

### Minute 3–4: The architecture flex
> "Behind the scenes: that order touched the REST gateway, the order router, the risk service, the ledger, the matching engine, and three message bus subjects. The matching engine is Liquibook in C++ — production-grade, headers-only library that's been benchmarked at over two million orders per second on a single thread.
> 
> Every fill produces a balanced double-entry ledger transaction. The position you see is computed by a separate position service that consumes the trade stream — same architecture institutional venues use."

### Minute 4–5: Lifecycle
> "Now let me show you the lifecycle. I'll switch to admin."
> 
> [Switch to admin tab; navigate to oracle]
> 
> "The June meeting just happened. In production, the resolution comes from signed attestations by independent oracle operators. For the demo, I'll resolve it manually."
> 
> [Resolve → CUT_25 (YES wins)]

### Minute 5–6: Settlement
> [Switch back to retail tab]
> 
> "Watch the contract state. CLOSED. RESOLVING. SETTLED. My position has paid out — I'm long 100 YES, $1 a contract, that's $100. Balance up by $100.
> 
> Eight positions settled total, total payout $1,247. Settlement took 1.4 seconds end to end."
> 
> [Open audit tab]
> 
> "And here's the audit chain. Every action — order, fill, hold, settlement, admin action — recorded."
> 
> "That's the demo. Happy to go deeper on any layer — matching engine internals, custody architecture, the path to DFSA authorization."

---

## Required hardware for the demo

- A laptop that can run docker-compose with 13 services + Postgres + NATS comfortably. **Minimum 16 GB RAM.** Recommended: 32 GB, dedicated dev laptop that's not also running a million tabs.
- Backup laptop with the same state.
- Both should have the recorded backup video saved locally.
- For in-person demos: a wired display adapter you've tested with your laptop.

---

## What can go wrong (and contingencies)

| Failure | Plan B |
|---|---|
| Docker won't start on demo laptop | Backup laptop |
| Backup laptop also fails | Play recorded video; explain it's a real recording, not a mockup |
| MM bots stop quoting mid-demo | Frontend has a button "Reset MM bots" only visible in DEMO_MODE; click it |
| Resolution doesn't trigger settlement | Reset, restart, switch to recorded video for the settlement scene |
| Investor asks question about something stubbed | Honest answer: "That's stubbed in the demo because we wanted to focus the engineering effort on the matching engine, ledger, and oracle. Post-funding, we replace it with [Fireblocks / multi-source attestor / HA setup] in months [X] of the roadmap." Hand them the production design doc. |

---

## Definition of Done for Phase 0

- [ ] All 13 services build and run from `make run`
- [ ] Demo script runs end to end in ≤ 7 minutes with zero manual intervention beyond the prescribed clicks
- [ ] `make reset` returns demo to a clean state in ≤ 30 seconds
- [ ] At least 2 friendly viewers have seen the demo and given feedback
- [ ] Backup video recorded and verified
- [ ] Production design doc handed off to follow-up technical investors

---

## After Phase 0: Phase 1 polish (Weeks 5–16)

Cap engineering at ~10 hours/week so you don't burn yourself out during fundraise.

**Acceptable polish work during fundraise:**
- New flagship contracts (5 → 8 → 12 markets to show breadth)
- Better data viz
- Additional MM bot strategies for visual variety
- Bug fixes from investor questions
- "Roadmap" page tied to specific feature commitments

**Not acceptable polish work during fundraise:**
- Real Fireblocks integration
- HA setup
- Real auth/security work
- Anything that touches the matching engine internals (you don't want to break a working demo)

Update `ROADMAP.md` every other Friday.
