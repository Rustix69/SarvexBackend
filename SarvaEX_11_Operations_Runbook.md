# 11 — Runbook

Operational guide for running the demo (Phase 0) and seed for production runbooks (Phase 2+).

---

## 1. First-time setup (developer onboarding)

### Prerequisites

- macOS 14+, Ubuntu 22.04+, or WSL2 with Ubuntu 22.04+
- Docker Desktop 4.30+ (or Docker Engine 26+ with docker-compose v2)
- At least 16 GB RAM, 8 CPU cores recommended
- Git, Make, Go 1.22+, Node 20+, Python 3.11+, CMake 3.20+, a modern C++ compiler (gcc-12+ or clang-16+)
- `buf` (proto management): `brew install bufbuild/buf/buf`
- `golang-migrate`: `brew install golang-migrate`

### Clone and bootstrap

```bash
git clone --recurse-submodules git@github.com:sarvaex/sarvaex.git
cd sarvaex
cp .env.example .env

# Generate proto bindings
./scripts/proto-gen.sh

# Build everything
make build

# Bring up infra + services
make run

# Wait ~30 seconds, then seed
make seed

# Open the app
open http://localhost:3000
```

### Demo credentials

```
Retail:        retail@demo.sarvaex.com / demo1234
Institutional: inst@demo.sarvaex.com / demo1234
Market Maker:  mm@demo.sarvaex.com / demo1234
Admin:         admin@demo.sarvaex.com / demo1234
```

---

## 2. Common commands

```bash
make build                # Build all services + frontend
make run                  # docker-compose up -d
make stop                 # docker-compose down (keeps volumes)
make reset                # docker-compose down -v && make run && make seed
make seed                 # Apply demo seed data
make logs                 # Tail all service logs
make logs SVC=me-core     # Tail one service
make test                 # Run all tests
make test-go              # Go tests only
make test-me              # me-core C++ tests only
make proto                # Regenerate proto bindings
make migrate              # Apply DB migrations
make migrate-create NAME=add_foo SVC=ledger  # Create a new migration
```

---

## 3. Service ports (demo)

| Service | Port | Protocol |
|---|---|---|
| Frontend | 3000 | HTTP |
| gw-rest | 8080 | HTTP |
| gw-ws | 8081 | WS |
| order-router | 50061 | gRPC |
| risk-svc | 50062 | gRPC |
| ledger-svc | 50063 | gRPC |
| me-core | 50051 | gRPC |
| position-svc | 50064 | gRPC |
| refdata-svc | 50065 | gRPC |
| oracle-svc | 50066 | gRPC |
| settlement-svc | 50067 | gRPC |
| audit-svc | 50068 | gRPC |
| admin-svc | 50069 | gRPC |
| Postgres | 5432 | postgres |
| NATS | 4222 | nats |
| Redis | 6379 | redis |
| Prometheus metrics | each svc + 10000 | HTTP |
| Health | each svc + 20000 | HTTP |

In production these all become internal cluster services; only frontend, gw-rest, and gw-ws are public.

---

## 4. Demo runbook (live investor demo)

**Before the demo (T-30 min):**

```bash
# Fresh state
make reset

# Wait for services to be healthy
./scripts/wait-for-healthy.sh
# (loops checking /health/ready on every service; exits 0 when all green)

# Seed demo data + start MM bots
make seed
docker-compose start mm-bots

# Verify everything by running smoke test
./scripts/demo-smoke-test.sh
# (places a test order, verifies fill, cancels, verifies refund)

# Open the URLs in tabs
open http://localhost:3000           # Trading UI (retail user)
open http://localhost:3000/admin     # Admin (admin user — needs separate login)
```

**During the demo, troubleshooting:**

| Issue | Quick fix |
|---|---|
| Order book empty | `docker-compose restart mm-bots` |
| Order rejected with "BOOK_NOT_FOUND" | Contract isn't OPEN — admin: transition state to OPEN |
| me-core seems frozen | `docker-compose restart me-core` (loses books; admin needs to re-list contracts; ideally don't let this happen mid-demo) |
| Frontend not updating | Hard refresh; if persistent, `docker-compose restart gw-ws` |
| Latency display showing nothing | Wait 5s; if persistent, ignore |

**After the demo:**

```bash
make reset    # back to clean state for next demo
```

---

## 5. Service-by-service operations

### 5.1 me-core

**Restart behavior:** rebuilds book from `orders.orders` WHERE status IN (OPEN, PARTIAL). Takes 1-5 seconds for the demo.

**Production:** restores from snapshot + journal. RTO target 30s cold, 5s failover.

**Common operations:**
```bash
# View resting orders count per contract
docker-compose exec me-core grpcurl -plaintext localhost:50051 \
    sarvaex.v1.MatchingEngine/GetBookSnapshot
# (use the snapshot to inspect)

# Force-cancel everything on a contract (admin path)
# Through admin-svc → me-core CancelOrder for each order
```

**Failure modes:**
- **Restart loop:** likely a corrupt order in DB. Inspect `docker logs me-core`; manually fix the offending row.
- **Hung sequencer:** check `me-core_inbound_ring_depth` metric. If > 1000 sustained, sequencer is stuck. Last resort: restart.

### 5.2 ledger-svc

**Critical invariant:** for any timestamp `T`, sum over all entries with `posted_at <= T` of `signed_amount` per currency = 0. **Production cron checks this hourly.**

**Common operations:**

```bash
# Credit a demo user (admin only, demo mode)
curl -X POST http://localhost:8080/v1/admin/deposits/credit \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    -d '{"user_id": "u_retail_1", "amount_micro_usdc": 1000000000, "note": "demo top-up"}'

# Check user balance
psql -d sarvaex -c "SELECT * FROM ledger.user_balances WHERE user_id='u_retail_1'"

# Audit a hold
psql -d sarvaex -c "SELECT * FROM ledger.holds WHERE hold_id='<id>'"
```

**Failure modes:**
- **Unbalanced transaction:** deferred trigger catches this at commit. Whoever called `PostTransaction` gets an error; investigate.
- **Drift in production:** likely a missed publish to NATS (a fill posted to ledger but not to position-svc). Reconcile by replaying fills.

### 5.3 order-router

**Common operations:**

```bash
# Check pending orders
psql -d sarvaex -c "SELECT order_id, status, ticker, count, filled_count FROM orders.orders WHERE status IN ('OPEN', 'PARTIAL') ORDER BY created_at DESC LIMIT 20"

# Force-cancel a stuck order (manual escape hatch)
grpcurl -plaintext -d '{"order_id":"<id>","user_id":"<id>"}' \
    localhost:50061 sarvaex.v1.OrderRouter/CancelOrder
```

**Failure modes:**
- **ME timeout on submit:** ledger hold not released. `order-router` has retry logic; if it fails permanently, the hold lingers. Admin can release via `Ledger.ReleaseHold` directly with a manual idempotency key.

### 5.4 risk-svc

Mostly stateless. Reads user limits + working orders summary. Restart is safe.

**Common operations:**

```bash
# Update a user's limits
grpcurl -plaintext -d '{...}' localhost:50062 sarvaex.v1.Risk/UpdateUserLimits

# Rebuild working orders summary (if it drifts)
./scripts/rebuild-risk-summary.sh
# (queries orders.orders, recomputes summary)
```

### 5.5 oracle-svc + settlement-svc

**Common operations:**

```bash
# Resolve a contract (demo)
curl -X POST http://localhost:8080/v1/admin/contracts/RBI-JUN26-CUT25/resolve \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    -d '{"categorical_value": "YES", "justification": "Demo resolution"}'

# Check settlement status
psql -d sarvaex -c "SELECT * FROM settlement.settlements WHERE ticker='RBI-JUN26-CUT25'"

# View payouts
psql -d sarvaex -c "SELECT * FROM settlement.settlement_payouts WHERE ticker='RBI-JUN26-CUT25'"
```

**Failure modes:**
- **Settlement stuck IN_PROGRESS:** worker probably crashed. Restart settlement-svc; it picks up from where it left off (idempotency keys protect against double-paying).
- **Settlement complete but UNSETTLED_TRADES balance non-zero:** drift. Investigate which positions were missed.

---

## 6. Database operations

### Backups (production)

```bash
# Full RDS snapshot (managed by AWS Backup, daily)
# WAL archiving to S3 for PITR
# Manual snapshot before major changes:
aws rds create-db-snapshot --db-instance-identifier sarvaex-prod --db-snapshot-identifier pre-launch-2026-08-01
```

### Restore from PITR

```bash
aws rds restore-db-instance-to-point-in-time \
    --source-db-instance-identifier sarvaex-prod \
    --target-db-instance-identifier sarvaex-restore-test \
    --restore-time 2026-08-01T15:30:00Z
```

### Schema migrations

Always:
1. Test migration on a copy of prod data
2. Apply during low-traffic window
3. Use `golang-migrate up --steps=1` to apply one migration at a time
4. Have rollback migration ready (`golang-migrate down --steps=1`)
5. For ALTER TABLE on `ledger.entries`: NEVER. The table is append-only and partitioned by month; new schemas apply to new partitions.

---

## 7. Incident response (Phase 2+)

### Severity definitions

| Severity | Definition | Response time | Examples |
|---|---|---|---|
| P0 | User funds at risk or system down | 15 min ack | Ledger drift, ME down, custody compromise |
| P1 | Major degradation, no fund risk | 1 hour ack | ME failover triggered, NATS partial outage, settlement delayed |
| P2 | Minor degradation | 4 hour ack | Single endpoint slow, monitoring blind spot, MM bot down in demo |
| P3 | Cosmetic / single-user impact | 24 hour ack | UI bug, single user can't log in |

### On-call rotation (production)

- 24/7 primary on-call with secondary backup
- Weekly rotation among 4 engineers minimum
- PagerDuty escalation policy: primary → secondary (15 min) → engineering manager (30 min) → CTO (45 min)

### Postmortem requirement

Every P0 and P1 requires a postmortem within 5 business days, posted to `docs/postmortems/`. Blameless. Action items tracked to completion.

---

## 8. Demo reset script (detailed)

```bash
#!/usr/bin/env bash
# scripts/reset-demo.sh — bring demo to clean state in ≤30s

set -euo pipefail

echo "→ Stopping services..."
docker-compose stop me-core order-router risk-svc settlement-svc oracle-svc audit-svc admin-svc mm-bots gw-rest gw-ws

echo "→ Truncating volatile tables..."
psql -d sarvaex <<SQL
SET search_path TO public;

TRUNCATE TABLE orders.fills, orders.orders CASCADE;
TRUNCATE TABLE position.positions, position.position_history, position.open_interest CASCADE;
TRUNCATE TABLE risk.working_orders_summary CASCADE;
TRUNCATE TABLE ledger.entries, ledger.transactions, ledger.holds CASCADE;
-- Note: we keep ledger.accounts so account_codes are preserved
TRUNCATE TABLE oracle.attestations, oracle.resolutions CASCADE;
TRUNCATE TABLE settlement.settlement_payouts, settlement.settlements CASCADE;
TRUNCATE TABLE audit.events CASCADE;
TRUNCATE TABLE refdata.contract_state_history CASCADE;

-- Reset contract states
UPDATE refdata.contracts SET state='OPEN', updated_at=now();
SQL

echo "→ Restarting services..."
docker-compose start gw-rest gw-ws audit-svc admin-svc oracle-svc settlement-svc
docker-compose start refdata-svc risk-svc ledger-svc position-svc order-router me-core

echo "→ Waiting for healthy..."
./scripts/wait-for-healthy.sh

echo "→ Seeding demo data..."
./scripts/seed-demo-data.sh

echo "→ Starting MM bots..."
docker-compose start mm-bots

echo "✓ Reset complete in $SECONDS seconds"
```

---

## 9. Seed script

```bash
#!/usr/bin/env bash
# scripts/seed-demo-data.sh — credit each demo user $1k, ensure contracts open

set -euo pipefail

ADMIN_TOKEN=$(curl -s -X POST http://localhost:8080/v1/auth/login \
    -d '{"email":"admin@demo.sarvaex.com","password":"demo1234"}' | jq -r .token)

for user in u_retail_1 u_inst_1 u_mm_1; do
    curl -s -X POST http://localhost:8080/v1/admin/deposits/credit \
        -H "Authorization: Bearer $ADMIN_TOKEN" \
        -H "Idempotency-Key: seed-$user" \
        -d "{\"user_id\":\"$user\",\"amount_micro_usdc\":1000000000,\"note\":\"demo seed\"}"
    echo "→ Credited $user"
done

echo "✓ Seed complete"
```

---

## 10. Monitoring (demo)

The demo doesn't have full Prometheus + Grafana; instead:

```bash
# Quick health check across all services
./scripts/check-health.sh

# Tail audit log in real-time
docker-compose logs -f audit-svc | grep AuditEvent

# Order rate per second (last 60s)
psql -d sarvaex -c "SELECT count(*) FROM orders.orders WHERE created_at > now() - interval '60 seconds'"
```

Production adds full observability (see Phase 2C).

---

## 11. Security checklist (production)

Before opening to real users:

- [ ] All services run as non-root in containers
- [ ] Read-only root filesystem where feasible
- [ ] No secrets in environment variables (all from Secrets Manager)
- [ ] All gRPC internal: mTLS
- [ ] All HTTPS external: TLS 1.3 only
- [ ] WAF rules enabled, tested
- [ ] Rate limiting verified at scale
- [ ] Pen-test findings remediated
- [ ] Bug bounty live
- [ ] Backups tested with restore drill in last 30 days
- [ ] Fireblocks API keys rotated
- [ ] KMS key access reviewed
- [ ] All admin actions require dual approval
- [ ] Audit chain verified continuously
- [ ] No PII in logs
- [ ] Data retention policy enforced

---

## 12. Useful one-liners

```bash
# How much real cash is sitting in the system?
psql -d sarvaex -c "
SELECT
  account_code,
  SUM(CASE WHEN direction='CR' THEN amount_micro_usdc ELSE -amount_micro_usdc END) / 1e6 AS balance_usdc
FROM ledger.entries e JOIN ledger.accounts a USING(account_id)
WHERE account_code LIKE 'LIAB:USER:%:CASH'
GROUP BY account_code ORDER BY balance_usdc DESC LIMIT 20"

# Top 10 active markets by order count today
psql -d sarvaex -c "
SELECT ticker, count(*) AS orders FROM orders.orders
WHERE created_at::date = current_date GROUP BY ticker ORDER BY orders DESC LIMIT 10"

# Recent fills
psql -d sarvaex -c "
SELECT ticker, price_ticks, count, ts FROM orders.fills ORDER BY ts DESC LIMIT 20"

# Open interest per market
psql -d sarvaex -c "
SELECT ticker, total_open_long, total_open_short FROM position.open_interest ORDER BY total_open_long DESC LIMIT 10"

# Recent audit events
psql -d sarvaex -c "
SELECT ts, service, type, actor, subject FROM audit.events ORDER BY event_seq DESC LIMIT 20"
```

---

## 13. Things that should never happen (and what to do if they do)

| Event | What it means | Immediate response |
|---|---|---|
| Ledger drift > $1 | Someone bypassed double-entry, or a publish failed | P0. Freeze withdrawals. Investigate. |
| Audit chain break | Tamper attempt or bug | P0. Lock down. Investigate. Preserve evidence. |
| ME running but stale (no fills for >5min on a live contract) | Sequencer stuck | P1. Restart ME (or failover in prod). |
| Fireblocks reconciliation mismatch | Custody drift | P0. Pause deposits/withdrawals. Manual reconciliation. |
| User submits 1000 orders in 1 second | Either a bug or attack | P1. Rate limit kicks in. Investigate user. |
| Settlement completes but UNSETTLED_TRADES > $1 | Missed payouts | P0. Investigate. Manually credit missed positions. |
| Oracle proposes resolution with no quorum | Bug in oracle workflow | P1. Block resolution from finalizing. Investigate. |

Every one of these has a documented response in production runbooks. Demo runbook is lighter, but principle is same: **stop, investigate, fix, postmortem**.

---

## 14. Where to find help

| Topic | Document |
|---|---|
| Architecture overview | `01_foundations.md` |
| Wire protocol | `02_protobufs.md` |
| Database schemas | `03_databases.md` |
| me-core internals | `04_me_core.md` |
| Service designs | `05_services.md` |
| Oracle / Settlement | `06_oracle_settlement.md` |
| Gateways | `07_gateways.md` |
| Frontend | `08_frontend.md` |
| Build plan | `09_demo_phase.md` |
| Production plan | `10_production_phase.md` |
| This document | `11_runbook.md` |

Plus:
- `ROADMAP.md` — current state, updated bi-weekly
- `docs/adr/` — Architecture Decision Records for major changes
- `docs/postmortems/` — post-incident analyses
- `docs/api/` — generated API reference from protos
