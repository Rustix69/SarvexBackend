# 10 — Production Phase (Months 5–14)

This is what happens after the seed closes. The architecture doesn't change; we replace stubs with production implementations service by service. **No service boundary moves. No protobuf changes. No data model migration.**

Calendar assumes seed closes at the end of Month 4 and we have a CTO + 3 engineers + 1 DevOps from Month 5.

---

## Phase 2A — Money is real (Months 5–6)

The single biggest risk reduction: getting from "fake credit" to real USDC. Everything else can wait.

### Month 5

**Team:**
- 1 backend (custody integration lead)
- 1 backend (ledger hardening)
- 1 DevOps (infra setup)
- CTO (oversight + KYC vendor selection)

**Workstreams:**

#### W1: Fireblocks integration

- [ ] Fireblocks account setup (Workspace tier, MPC-CMP, with 2 hot vaults, 1 warm vault, 1 cold vault)
- [ ] `services/wallet-watcher/` new service (Rust or Go) — subscribes to chain RPCs (Alchemy + QuickNode redundancy) for USDC `Transfer` events on Ethereum and Arbitrum
- [ ] Deposit attribution: chain memo (preferred) or per-user deposit address mapping
- [ ] `ledger-svc` learns to credit `LIAB:USER:<id>:CASH` from real deposits, debiting `ASSET:HOTWALLET:<chain>`
- [ ] `services/withdrawal-svc/` new service — queues withdrawal requests, calls Fireblocks Withdraw API with policy engine
- [ ] Withdrawal policy: max $X/day per user without manual review, escalates above threshold
- [ ] Hourly reconciliation cron: sums of `ASSET:HOTWALLET:*` balances must equal Fireblocks API reported balances ± epsilon
- [ ] Drift alerting: any deviation > $1 fires PagerDuty

#### W2: Ledger hardening

- [ ] Add tamper-evident hash chain to `audit.events` (production: each row's `hash = SHA256(prev_hash || canonical(row))`)
- [ ] Verify chain integrity nightly; fail loud if break detected
- [ ] Add retention policy: `audit.events` kept indefinitely (regulatory); `orders.fills` kept 7 years
- [ ] Migrate `ledger.entries` to a partitioned table (by month) — same schema, just partitioned for performance at scale
- [ ] Add Postgres logical replication to a read replica for analytics queries

#### W3: KYC vendor

- [ ] Vendor selection: shortlist Sumsub, Persona, Veriff. Pick one based on DIFC + India coverage.
- [ ] `services/kyc-svc/` new service — wraps vendor API
- [ ] `users.users` adds `kyc_status`, `kyc_documents_ref`
- [ ] Frontend: KYC flow embedded into signup
- [ ] Tier 1 (verified ID) unlocks $5k limits; Tier 2 (income proof) unlocks higher

### Month 6

**Continued:**

- [ ] Withdrawal policy engine: 2-of-3 admin approval for withdrawals > $50k
- [ ] Per-user deposit/withdrawal limits enforced
- [ ] Anti-fraud: velocity checks, blocklist integration (OFAC, UN sanctions)
- [ ] Customer support tooling: search by user, view full audit timeline
- [ ] Bug bounty program launched with HackerOne (small payouts during build, ramp up at launch)

**Exit criteria for Phase 2A:**
- Real USDC moves into and out of the system
- Reconciliation runs hourly with $0 drift
- KYC enforced for any non-test user
- $100k of real money sitting in test mode without issue

---

## Phase 2B — Matching engine becomes production-grade (Months 7–8)

The matching engine is the highest-stakes service. Phase 2B turns it from "demo single-process" into "production with sub-5-second failover."

### Month 7

**Team:**
- 1 senior C++ engineer (ME core)
- 1 backend (security)
- 1 DevOps (HA + observability)

#### W1: Journal writer

- [ ] Replace `NoOpJournal` with `JetStreamJournal`
- [ ] Each command appended to NATS JetStream subject `me.journal.<shard>` before book mutation
- [ ] JetStream cluster R3 (3 replicas, sync replication, durability `Memory + File`)
- [ ] Journal segments rotate at 1 GB; old segments compacted into snapshots
- [ ] Sequencer waits for JetStream `Ack` before applying to book; this is the new critical-path latency
- [ ] Batch `fdatasync`: up to 200µs window or 64 commands, whichever first

#### W2: Snapshotter

- [ ] Snapshotter thread runs every 30 seconds
- [ ] Atomically copy book state (use Liquibook's iterator + freeze for milliseconds)
- [ ] Serialize via Cap'n Proto (fast and zero-copy on restore)
- [ ] Write to local NVMe; async upload to S3 `s3://sarvex-snapshots/me-core/<shard>/<seq>.capnp`
- [ ] Retention: keep last 24 hours of snapshots locally, 30 days in S3

#### W3: Cold restore from snapshot + journal

- [ ] On startup: download latest snapshot from S3 (or local NVMe)
- [ ] Deserialize → populate books
- [ ] Tail JetStream subject from `snapshot.global_seq + 1`
- [ ] Apply commands until caught up
- [ ] Open gRPC port
- [ ] Restore time target: 30 seconds for a shard with 100k resting orders

### Month 8

#### W1: Hot-standby replica

- [ ] Two `me-core` pods per shard: primary and standby
- [ ] Standby runs in "replica mode": tails journal continuously, applies to books, never writes
- [ ] Leader election via Kubernetes Lease object (15s lease, 5s renewal)
- [ ] On primary loss: standby promotes after draining last commands from journal, advertises leader, takes over gRPC service traffic
- [ ] RTO target: 5 seconds. RPO: 0 (journal is durable in JetStream R3 before primary acks).

#### W2: Failover testing

- [ ] Chaos engineering setup (Chaos Mesh on EKS or Litmus)
- [ ] Test plan: kill primary me-core randomly during sustained 1k orders/sec load
- [ ] Measure RTO. Iterate until p99 < 5 seconds.
- [ ] Test AZ failure: kill all pods in one AZ; verify recovery in another.

#### W3: Performance hardening

- [ ] CPU pinning of sequencer thread (`pthread_setaffinity_np` + `isolcpus` boot param)
- [ ] Pre-allocated memory arena for orders/fills
- [ ] NUMA-aware allocation
- [ ] Tune NATS publisher batching for throughput
- [ ] Load test: 10,000 orders/sec sustained for 1 hour with p99 ack < 5 ms

#### W4: Security (in parallel)

- [ ] mTLS everywhere internal (cert-manager + SPIFFE identities)
- [ ] Ed25519 request signing for API keys (HTTP signatures style)
- [ ] OPA policies for service-to-service authorization
- [ ] AWS KMS for secret encryption at rest
- [ ] Secrets Manager for runtime secret injection
- [ ] Engage external pen-test firm for 4-week engagement starting Month 9

**Exit criteria for Phase 2B:**
- Matching engine sustains 10k orders/sec at p99 ack < 5 ms
- Failover RTO < 5 seconds across 100 chaos tests
- All internal traffic mTLS-encrypted
- All secrets in KMS, none in environment variables
- Pen-test scoped and scheduled

---

## Phase 2C — Production infrastructure (Months 9–10)

### Month 9

**Team:** 1 DevOps lead, 1 SRE, plus everyone helping with their service's K8s migration.

#### W1–2: EKS setup

- [ ] AWS account `me-central-1` (Bahrain) for DIFC alignment
- [ ] VPC with public/private subnets across 3 AZs
- [ ] EKS cluster with managed node groups
- [ ] Postgres RDS Multi-AZ (PostgreSQL 16) + cross-region read replica
- [ ] NATS cluster: 3 nodes, JetStream enabled, R3 replication
- [ ] Redis ElastiCache cluster with Sentinel
- [ ] S3 buckets for snapshots, audit archive, backups
- [ ] AWS WAF + CloudFront in front of public API

#### W3: Service deployment

- [ ] Helm charts per service
- [ ] CI/CD with GitHub Actions → ECR → EKS
- [ ] Argo CD or Flux for GitOps continuous deployment
- [ ] Blue/green deployment for ME (with extra safeguards)
- [ ] Rolling deployment for stateless services

#### W4: Observability

- [ ] Prometheus (managed via AMP) scraping all `/metrics` endpoints
- [ ] Grafana dashboards: one per service, plus business dashboards (orders/sec, OI per market, etc.)
- [ ] Tempo for distributed traces; all gRPC instrumented with OTel
- [ ] Loki for logs; structured JSON logs ingested from stdout
- [ ] PagerDuty integration for critical alerts
- [ ] Alert rules: ME availability < 99.95%, ledger drift > $1, oracle quorum failure, etc.

### Month 10

#### W1: Pen-test

- [ ] 4-week external pen-test engagement
- [ ] Scope: API endpoints, authentication, authorization, infrastructure
- [ ] Triage findings; fix critical and high within 2 weeks
- [ ] Retest

#### W2–3: Load testing

- [ ] Build `tools/load-test/` (Go service that simulates trading at scale)
- [ ] Test: 10,000 orders/sec for 24 hours; verify no degradation, no memory leaks
- [ ] Test: 5,000 concurrent WSS connections; verify no message drops
- [ ] Test: 1 million orders in a single day; verify Postgres performance + retention

#### W4: Disaster recovery drill

- [ ] Practice: drop the entire production region; verify backup region can serve traffic
- [ ] Practice: drop Postgres primary; verify failover works
- [ ] Practice: corrupt me-core snapshot; verify recovery from older snapshot + journal works
- [ ] Document everything in runbooks (see `11_runbook.md`)

**Exit criteria for Phase 2C:**
- Production EKS environment running with full observability
- Pen-test findings remediated
- Load tests pass at production targets
- Disaster recovery drilled successfully

---

## Phase 3 — Launch readiness (Months 11–14)

### Month 11: Oracle hardening

- [ ] Replace ADMIN-only oracle with full `MULTI_SOURCE_ATTEST` policy
- [ ] Build attestor framework: separate processes per data source
- [ ] Attestor implementations:
  - Web scraper attestor (for press releases, with HTML hash verification)
  - API attestor (for sources with stable APIs)
  - Chainlink integration attestor (read prices from CL feeds for select markets)
- [ ] Challenge window UI: live countdown, dispute history, attestor reputation
- [ ] Attestor key management via KMS

### Month 12: LP/MM tier features

- [ ] Bulk order endpoint: submit/cancel/amend up to 100 orders per request
- [ ] Mass-cancel: cancel all orders for a user or contract
- [ ] Cancel-on-disconnect (LP feature): server cancels all orders on WS disconnect
- [ ] Post-only with self-trade blocking
- [ ] WebSocket order entry (lower latency than REST for LPs)
- [ ] FIX gateway prototype (Phase 4 if not Phase 3 — depends on LP demand)

### Month 12–13: Beta program

- [ ] Recruit 5–10 friendly counterparties (degens with discipline, prop shops, LPs)
- [ ] Deploy to production with low limits ($10k caps)
- [ ] Daily ops standup; weekly retro
- [ ] Iterate on UX, feature gaps, performance

### Month 13: Compliance & DFSA prep

- [ ] Audit chain integrity verification automated and gating each release
- [ ] Compliance reports: daily settlement reports, weekly trade reports, monthly user activity
- [ ] DFSA application drafting (with regulatory counsel)
- [ ] Internal compliance officer hired (Phase 3 hire)
- [ ] Transaction monitoring: rules-based + analyst review for suspicious activity
- [ ] OFAC and sanctions screening (continuous, not just at onboarding)

### Month 14: Soak + launch

- [ ] 30-day soak test: production system running with beta traffic + synthetic load
- [ ] Zero P0 incidents target
- [ ] All P1 incidents triaged with documented postmortems
- [ ] Runbooks tested by on-call rotation
- [ ] Public launch (waitlist-first, then open)

---

## What we hire and when

### Month 5 (just after close)
- CTO (probably already in place or first hire)
- Senior backend × 2 (one custody, one ledger)
- Senior DevOps / SRE
- Compliance officer (part-time, contracted)

### Month 7
- Senior C++ engineer (ME-focused)
- Senior backend (security)
- Frontend engineer (full-time on web)

### Month 9
- Junior SRE
- QA engineer (for load + chaos testing)

### Month 11
- Senior backend (oracle / data engineering)
- Compliance officer (full-time)
- Product / customer-facing role

### Month 13
- Customer support × 2
- BD / partnerships lead

**Total headcount by launch: ~12–15 people**

---

## Budget envelope (engineering only, monthly run rate)

| Month | Headcount | Approx. monthly cost (USD, fully loaded) |
|---|---|---|
| 5–6 | 5 | $80k |
| 7–8 | 7 | $115k |
| 9–10 | 9 | $145k |
| 11–12 | 12 | $190k |
| 13–14 | 15 | $235k |

Plus infrastructure (Fireblocks subscription, AWS, KYC vendor, oracle data sources, security tooling) at ~$30k/month by Month 14.

Total Phase 2–3 engineering + infra: ~$2.0M–$2.5M of the seed round used for build.

---

## Phase boundary discipline

Each phase has a **gate review**. Don't move to the next phase until:

### Phase 2A → 2B gate
- Real USDC moves end-to-end with zero drift over 30 days
- KYC enforced
- Withdrawal policy engine working
- $100k test deposits processed without intervention

### Phase 2B → 2C gate
- 10k orders/sec sustained for 24h
- Failover RTO < 5s verified 100 times
- mTLS everywhere
- Pen-test scheduled

### Phase 2C → 3 gate
- EKS environment production-ready
- Pen-test findings remediated
- Disaster recovery drilled
- Compliance reporting automated

### Phase 3 → Launch gate
- 30-day soak with zero P0
- Beta cohort happy
- DFSA pre-launch dialogue progressing
- Compliance officer satisfied with controls

These gates exist to prevent the "we're 80% done with everything" problem that kills exchanges.

---

## What we deliberately don't build before launch

- **Mobile apps.** Web only. Mobile is Phase 4 (post-launch, demand-driven).
- **Cross-margin.** Phase 4 risk model redesign.
- **Options on options.** Out of scope.
- **Tokenization / on-chain settlement.** Unless a regulatory + product reason emerges, we stay book-entry USDC.
- **DAO / token.** No.
- **FIX gateway for retail.** LP/MM only, and only if demand justifies it.
- **Multi-currency.** USDC only at launch. USDT, EUR, others are Phase 4.
- **Margin trading.** Not in this design at all. Different product entirely.

---

## Continuous activities throughout production phase

- Bi-weekly ROADMAP.md updates
- Weekly engineering all-hands
- Monthly retrospectives
- Quarterly architecture review (any service drifting from design?)
- Quarterly business review (any feature direction shift?)
- Continuous documentation: every new service or major component documented before merging
- Continuous testing: unit + integration coverage > 80% per service, enforced in CI

---

## The big picture

The demo (Phase 0) proves the architecture works.
Phase 1 keeps the demo healthy during the raise.
Phase 2 replaces every stub with production-grade implementations behind unchanged interfaces.
Phase 3 hardens and prepares for launch.

By Month 14, the same Liquibook code that ran in the demo is running in production. The same protobufs. The same Postgres schemas. The same matching protocol. What's different: real USDC, real custody, real HA, real oracle, real auth, real infrastructure.

The architecture didn't change. We just filled in the parts that were stubbed.
