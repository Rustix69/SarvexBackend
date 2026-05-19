# 05 — Core Services

Detailed designs for: `order-router`, `risk-svc`, `ledger-svc`, `position-svc`, `refdata-svc`, `audit-svc`, `admin-svc`.

Each section follows the same structure: responsibility, demo implementation, production implementation, key algorithms, failure modes.

---

## 1. `order-router`

### 1.1 Responsibility

Orchestrate the order entry pipeline: validate → risk check → place hold → submit to ME → respond. Persist orders and fills. The single synchronous flow that ties everything together.

### 1.2 Inbound flow

```
gw-rest -> order-router.SubmitOrder()
  1. Validate request shape (proto validation)
  2. Look up contract spec from refdata (Redis cache, 60s TTL)
  3. Validate: tick alignment, price range, qty, contract state == OPEN
  4. Generate order_id (Snowflake)
  5. INSERT INTO orders.orders (status='PENDING') -- durable before any side effect
  6. Risk.PreTradeCheck() -> {approved, required_hold_micro_usdc}
     - On reject: UPDATE orders SET status='REJECTED'; return
  7. Ledger.PlaceHold(idempotency_key='place_hold:' + order_id, amount=required_hold) -> {hold_id}
     - On fail: UPDATE orders SET status='REJECTED' reason='INSUFFICIENT_FUNDS'; return
  8. MatchingEngine.SubmitOrder(order_id, hold_id, ticker, side, action, price, qty, flags, stp)
     - RESOURCE_EXHAUSTED before enqueue: release hold; mark REJECTED/ENGINE_QUEUE_FULL
     - DEADLINE_EXCEEDED after enqueue: keep order PENDING; do not release hold; start reconciliation
     - ME reject: release hold idempotently; mark REJECTED with ME reject_code
     - ME accept: mark OPEN/PARTIAL/FILLED using returned global_seq/contract_seq
  9. For each returned fill: in one orders DB tx persist fills, update both orders, insert fill_posting_outbox rows
 10. Fill poster drains outbox and posts ledger transactions idempotently by fill_id
 11. Return ack. If the ME outcome is unknown, return the PENDING order with reject_code='ACK_UNKNOWN'
```

A timeout after enqueue is deliberately not terminal. The reconciliation worker uses `order_id`, `MatchingEngine.StreamExecutions`, and `OrderRouter.GetOrder` state to resolve `PENDING` orders. Holds are released only after a terminal reject/cancel/expiry is known.

### 1.3 Order ID generation

Snowflake. 64-bit packed:
- 1 bit unused (always 0; keeps value positive)
- 41 bits ms since `2026-01-01T00:00:00Z` (good for ~69 years)
- 10 bits router pod ID (0..1023)
- 12 bits sequence within ms (0..4095)

```go
type Snowflake struct {
    epoch     time.Time
    podID     int64
    lastMs    int64
    seq       int64
    mu        sync.Mutex
}

func (s *Snowflake) Next() string {
    s.mu.Lock()
    defer s.mu.Unlock()
    now := time.Now().UTC()
    ms := now.Sub(s.epoch).Milliseconds()
    if ms == s.lastMs {
        s.seq++
        if s.seq >= 4096 {
            // wait until next ms
            for ms == s.lastMs {
                time.Sleep(50 * time.Microsecond)
                ms = time.Now().UTC().Sub(s.epoch).Milliseconds()
            }
            s.seq = 0
        }
    } else {
        s.seq = 0
        s.lastMs = ms
    }
    id := (ms << 22) | (s.podID << 12) | s.seq
    return strconv.FormatInt(id, 10)
}
```

Demo: single pod, podID=0. Production: derives podID from `K8S_POD_NAME` ordinal.

### 1.4 Idempotency

Three layers, all required:

| Layer | Key | TTL | Storage |
|---|---|---|---|
| Request-level | `Idempotency-Key` header + endpoint + user | 24h | Demo: in-memory LRU; production: Redis |
| Client-order-level | `(user_id, client_order_id)` | forever | Postgres UNIQUE constraint |
| ME-level | `order_id` | session/demo; journal/prod | me-core LRU in demo; journal in prod |

`POST /v1/orders` should require either a caller-supplied `client_order_id` or generate one at the gateway and echo it back. If a client retries with the same `Idempotency-Key`, return the captured response. If it retries with a different request key but the same `client_order_id`, the Postgres unique constraint wins and the router returns the existing order's current state.

### 1.5 Post-fill ledger work

When the ME returns fills, the router must make fill persistence durable before any async ledger work:

1. Persist each immutable fill to `orders.fills` with `global_seq`, `contract_seq`, both users, both orders, both hold IDs, side/action pairs, price, qty, fees.
2. Update both maker and taker `orders.orders` filled/remaining counters and statuses.
3. Insert one `orders.fill_posting_outbox` row per fill in the same Postgres transaction.
4. A fill-poster worker posts ledger transactions from the outbox using idempotency key `fill:<fill_id>`.
5. On success, mark `orders.fills.ledger_post_status='POSTED'`, persist `ledger_tx_id`, and mark the outbox row `POSTED`.

The outbox is required even in the demo. A Go channel may be used as a wake-up mechanism, but it is not the source of truth. On process restart, the worker drains all `PENDING`/retryable outbox rows ordered by `global_seq`.

If ledger posting fails after retries, the contract is halted and settlement is blocked until all fills up to the contract `close_global_seq` have `ledger_post_status='POSTED'`. This avoids the state where money moved on the book but not in the ledger.

In production, the same outbox contract remains valid. The worker may be split into a dedicated `fill-poster` service consuming JetStream, but `fill_id` idempotency and the `orders.fills` ledger status remain the cross-service truth.

Terminal non-fill events (`CANCELLED`, `EXPIRED`, `REJECTED`) are also idempotent by `order_id` + terminal reason. For `BOOK_CLOSED`, STP, user cancel, or admin cancel, order-router updates the order terminal state and calls `Ledger.ReleaseHold` for remaining unfilled quantity with a deterministic key such as `release:<order_id>:<reason_code>`.

### 1.6 Demo vs production differences

| Aspect | Demo | Production |
|---|---|---|
| Replicas | 1 pod | 4+ pods across AZs |
| ME timeout | 100 ms | 5 ms |
| Ledger timeout | 100 ms | 5 ms |
| Tracing | Simple log lines | OpenTelemetry to Tempo |
| Auth | Decode JWT, trust user_id | Same, but enforce mTLS to downstream |
| Idempotency cache | In-memory map | Redis |

### 1.7 File layout

```
services/order-router/
├── cmd/server/main.go
├── internal/
│   ├── server/        # gRPC server impl
│   ├── orderflow/     # The orchestration in §1.2
│   ├── snowflake/     # ID generator
│   ├── idem/          # idempotency cache (memory in demo, Redis in prod)
│   ├── refclient/     # Refdata gRPC client + Redis cache
│   ├── ledgerclient/  # Ledger gRPC client
│   ├── meclient/      # ME gRPC client
│   ├── riskclient/    # Risk gRPC client
│   └── persistence/   # Postgres
├── go.mod
└── Dockerfile
```

---

## 2. `risk-svc`

### 2.1 Responsibility

Pre-trade risk enforcement. Single-contract margin only in Phase 1 (no cross-margin until Phase 4).

### 2.2 The checks (in order)

```
PreTradeCheck(user_id, ticker, side, action, price_ticks, count):

1. Lookup user_limits — reject if frozen, KYC tier insufficient
2. Lookup contract from refdata (Redis cache) — reject if state != OPEN
3. Sanity:
   - count > 0
   - price_ticks in [contract.min, contract.max] OR price_ticks == 0 (market)
   - count <= contract.max_order_size
   - count * max_loss_per_contract <= user.max_order_size_micro_usdc
4. Compute required_hold_micro_usdc:
   - Binary BUY YES @ price_cents: max_loss = price_cents * count * 10000 (micro_usdc)
   - Binary BUY NO  @ price_cents: max_loss = price_cents * count * 10000
   - Binary SELL (= buy other side): max_loss = (100 - price_cents) * count * 10000
   - Scalar LONG @ price_ticks: max_loss = (price_ticks - lower_bound_ticks) * count * multiplier_micro_usdc / tick_normalize
   - Scalar SHORT @ price_ticks: max_loss = (upper_bound_ticks - price_ticks) * count * multiplier_micro_usdc / tick_normalize
5. Compute projected_position = current_position + working_orders_qty_signed + this_order_qty_signed
   - Reject if abs(projected_position) > contract.position_limit_per_user
6. Compute total_holds_after = current_holds + required_hold
   - We do NOT enforce balance check here — ledger.PlaceHold() does that authoritatively
7. Return {approved=true, required_hold_micro_usdc}
```

Balance enforcement lives in `ledger-svc`, not here. Reason: ledger is the only authoritative source. Doing the check twice introduces race conditions.

### 2.3 Working orders summary

`risk.working_orders_summary` is maintained from internal execution events:
- On accepted/open events from `exec.events`: increment `(user, ticker, side).total_qty` and `.total_max_loss_micro_usdc`.
- On fills from `exec.fills.<ticker>`: decrement filled quantity proportionally for both maker and taker orders.
- On cancel/expire/reject terminal events: decrement remaining reserved quantity.

This is eventually consistent in the demo. A user could submit a burst of orders faster than the summary catches up and briefly exceed a position limit. Ledger holds still prevent spending more cash than available, but position-limit enforcement is approximate. In production, `PreTradeCheck` uses an atomic Redis Lua reservation keyed by `(user,ticker,side)`; execution events then confirm or release the reservation.

### 2.4 Demo simplifications

- No Redis. All checks hit Postgres directly. Adds ~5ms; fine for demo loads.
- No rate-limit check (relies on gw-rest rate limiter only).
- No daily loss tracker (production adds this).

### 2.5 File layout

```
services/risk-svc/
├── cmd/server/main.go
├── internal/
│   ├── server/
│   ├── checks/        # Each check is a discrete function with tests
│   │   ├── sanity.go
│   │   ├── margin.go
│   │   ├── position_limit.go
│   ├── consumer/      # NATS consumer for exec.events / exec.fills.*
│   ├── refclient/
│   └── persistence/
├── go.mod
└── Dockerfile
```

---

## 3. `ledger-svc`

### 3.1 Responsibility

Authoritative double-entry ledger of all USDC movement. Every other service consults the ledger; the ledger consults no one.

### 3.2 Account naming convention

Account codes are structured strings:
```
ASSET:HOUSE:WALLET                  -- demo: virtual fake-deposit wallet
ASSET:HOTWALLET:ETH                 -- production: hot wallet per chain
ASSET:COLDWALLET:ETH                -- production
ASSET:WITHDRAWAL_INFLIGHT:ETH       -- production
LIAB:USER:<user_id>:CASH            -- what we owe user (available)
LIAB:USER:<user_id>:HOLDS           -- what we owe user (reserved against orders)
LIAB:HOUSE:UNSETTLED_TRADES:<ticker> -- escrow for committed trades
LIAB:UNALLOCATED_DEPOSIT             -- production: deposits awaiting attribution
REVENUE:FEES:TAKER
REVENUE:FEES:MAKER
REVENUE:SETTLEMENT_ROUNDING
```

User cash/holds accounts are created lazily on first use (or seeded at user creation).

### 3.3 Transaction patterns

**Demo deposit (admin-triggered):**
```
idem_key: "admin_deposit:<user>:<nonce>"
DR ASSET:HOUSE:WALLET           +1000 USDC
CR LIAB:USER:u_42:CASH          +1000 USDC
```

**Place hold (binary BUY YES @ 0.62 × 100 contracts = $62 max loss):**
```
idem_key: "hold:<order_id>"
DR LIAB:USER:u_42:CASH          −62
CR LIAB:USER:u_42:HOLDS         +62
```

**Commit hold (fill at 0.62 × 100 contracts = $62 fully consumed):**
```
idem_key: "commit_fill:<fill_id>"
DR LIAB:USER:u_42:HOLDS         −62        # release the held amount
CR LIAB:USER:u_42:CASH          +0         # net-zero (we're moving from holds to escrow)
                                            # Actually:
DR LIAB:USER:u_42:HOLDS         −62
CR LIAB:HOUSE:UNSETTLED_TRADES:RBI-JUN26-CUT25  +62
```

Wait — that's two debit-credit pairs in one transaction. Let me rewrite cleanly. A single balanced transaction with two pairs:
```
idem_key: "commit_fill:<fill_id>"
entries:
  DR LIAB:USER:u_42:HOLDS                          62
  CR LIAB:HOUSE:UNSETTLED_TRADES:RBI-JUN26-CUT25   62
```
Sum DR = 62; sum CR = 62; balanced. ✓

**Fill at lower price than holds covered (filled at 0.40 instead of 0.62):**

The hold was $62 (sized to max loss). Actual cost is $40. Refund $22 to cash:
```
idem_key: "commit_fill:<fill_id>"
entries:
  DR LIAB:USER:u_42:HOLDS                          62   # release entire hold
  CR LIAB:USER:u_42:CASH                           22   # refund overestimate
  CR LIAB:HOUSE:UNSETTLED_TRADES:RBI-JUN26-CUT25   40   # actual cost
```
Sum DR = 62; sum CR = 62. ✓

**Settlement (binary YES wins, user u_42 holds +100 YES):**
```
idem_key: "settlement:RBI-JUN26-CUT25:u_42"
entries:
  DR LIAB:HOUSE:UNSETTLED_TRADES:RBI-JUN26-CUT25  100   # $1 × 100 contracts
  CR LIAB:USER:u_42:CASH                          100
```

**Fees (taker pays 1 bp = 0.01% of notional, here $0.0040):**
```
idem_key: "fee:<fill_id>:taker"
entries:
  DR LIAB:USER:u_42:CASH                           4000 micro_usdc
  CR REVENUE:FEES:TAKER                            4000 micro_usdc
```

### 3.4 The `PostTransaction` implementation

```go
func (s *Server) PostTransaction(ctx, req) (*PostTransactionResponse, error) {
    // 1. Validate shape and balanced entries before opening the DB transaction.
    if err := assertBalanced(req.Entries); err != nil {
        return nil, status.Errorf(InvalidArgument, "unbalanced: %v", err)
    }

    // 2. Idempotency check. If a prior tx exists, return it without re-posting.
    if existing := lookupByIdempotencyKey(req.IdempotencyKey); existing != nil {
        return existing, nil
    }

    // 3. Single Postgres transaction. Read committed is acceptable only because
    // account rows are locked before balances are read; serializable is preferred
    // for production ledger hardening.
    tx := db.BeginTx(ctx, ...)
    defer tx.Rollback()

    // 4. Insert transaction row. Duplicate idempotency_key means another caller won.
    txID := insertLedgerTransaction(tx, req)

    // 5. Resolve/create accounts and lock account rows in deterministic account_id order.
    accountIDs := ensureAccountsAndLockInOrder(tx, req.Entries)

    // 6. For each entry, read the latest locked account balance/seq, compute the
    // new running balance, enforce account-level non-negative rules for user CASH
    // and HOLDS, and insert the entry.
    insertEntriesWithRunningBalances(tx, txID, req.Entries, accountIDs)

    // 7. Insert ledger_event_outbox row(s) in the same transaction.
    insertLedgerOutbox(tx, txID, req)

    // 8. Commit. Deferred trigger validates transaction balance.
    tx.Commit()

    // 9. Wake the NATS publisher. If publish fails, the outbox is retried.
    wakePublisher()
    return &PostTransactionResponse{TxId: strconv.FormatInt(txID, 10)}, nil
}
```

No caller may update a balance directly. Publishing to NATS is never the durability point; the committed ledger rows plus `ledger_event_outbox` are.

### 3.5 Sign convention for `running_balance_micro_usdc`

- `LIABILITY`, `EQUITY`, `REVENUE` accounts: credit-normal. CR increases balance, DR decreases.
- `ASSET`, `EXPENSE` accounts: debit-normal. DR increases balance, CR decreases.

```go
func signedAmount(e LedgerEntry, accountType AccountType) int64 {
    creditNormal := accountType == LIABILITY || accountType == EQUITY || accountType == REVENUE
    if (e.Direction == "CR") == creditNormal {
        return e.AmountMicroUsdc  // increases balance
    }
    return -e.AmountMicroUsdc
}
```

### 3.6 Holds

A hold is a logical reservation backed by ledger transactions and by idempotent hold operation rows. Place, release, and commit must be atomic with their ledger entries.

`PlaceHold(idempotency_key='place_hold:<order_id>')`:
1. Start one DB transaction.
2. Lock the user's CASH and HOLDS accounts in deterministic account order.
3. Verify available CASH is sufficient under the lock.
4. Insert the hold row if it does not exist.
5. Insert `hold_operations(idempotency_key, operation_type='PLACE')`.
6. Post the balanced ledger entries CASH -> HOLDS.
7. Commit.

`ReleaseHold(idempotency_key, hold_id, amount)`:
- If the idempotency key exists, return success.
- Lock the hold row and both accounts.
- Release only uncommitted/unreleased amount. `amount=0` means remaining uncommitted.
- Post HOLDS -> CASH, update `released_micro_usdc`, close the hold when fully consumed.

`CommitHold(idempotency_key, hold_id, commit_amount, release_amount, destination_account_code)`:
- If the idempotency key exists, return success.
- Lock the hold row and all involved accounts.
- Move `commit_amount` from user HOLDS to the destination account, usually `LIAB:HOUSE:UNSETTLED_TRADES:<ticker>`.
- Optionally release `release_amount` from HOLDS back to CASH in the same balanced transaction.
- Update `committed_micro_usdc` and `released_micro_usdc`; close the hold when fully consumed.

A fill normally commits both maker and taker holds with separate idempotency keys derived from the same fill, for example `commit:<fill_id>:maker` and `commit:<fill_id>:taker`, then posts fee entries under `fee:<fill_id>:maker|taker` or in the same fill transaction if the implementation keeps the entry set compact.

### 3.7 Demo vs production

| Aspect | Demo | Production |
|---|---|---|
| Deposit mechanism | Admin RPC `AdminCreditDeposit` | wallet-watcher service detects on-chain Transfer events |
| Withdrawal mechanism | Disabled (admin can debit) | Fireblocks integration with policy engine |
| Hot/warm/cold wallet split | Single `ASSET:HOUSE:WALLET` | Per-chain split with rebalancing |
| Reconciliation | None | Hourly cron vs Fireblocks API |
| HA | Single Postgres | Postgres primary + 2 sync replicas + 1 async cross-region |
| Replication | None | WAL streaming + PITR |
| Audit | Append-only schema | + tamper-evident hash chain in `audit_events` |

The schema is identical between demo and production. Production adds the `wallet-watcher` worker and the Fireblocks integration as new components — `ledger-svc`'s gRPC interface doesn't change.

### 3.8 File layout

```
services/ledger-svc/
├── cmd/server/main.go
├── internal/
│   ├── server/
│   ├── txposter/      # PostTransaction core logic
│   ├── holds/         # Hold lifecycle
│   ├── accounts/      # Account lookup + lazy creation
│   ├── persistence/
│   └── events/        # NATS publish to ledger.events
├── go.mod
└── Dockerfile
```

---

## 4. `position-svc`

### 4.1 Responsibility

Maintain user × contract positions. Fed exclusively from the internal fill stream (`exec.fills.*`) plus replay through `OrderRouter.ListFills`.

### 4.2 Consumer loop

`position-svc` consumes the private fill stream, not public market data. Public `md.trade.*` intentionally omits user IDs.

Demo consumer source:
- Primary: NATS Core subscription to `exec.fills.*`.
- Recovery: `OrderRouter.ListFills(from_global_seq=last_offset+1)` replay from `orders.fills`.

Production consumer source:
- JetStream durable consumer over `exec.fills.*`, with explicit ack after the DB transaction commits.

Processing rules:
1. Maintain `position.consumer_offsets('exec.fills')` as the last contiguous `global_seq` applied.
2. If a fill arrives with `global_seq > last+1`, pause live apply and replay the missing range through `OrderRouter.ListFills`.
3. If a fill arrives with `global_seq <= last`, skip only if `position.applied_fills(fill_id)` already exists; otherwise replay from the gap.
4. In one DB transaction, insert `applied_fills`, update maker/taker positions, update open interest, write `position_history`, and advance the offset.

```go
func applyFill(ctx, fill FillRecord) {
    tx := db.Begin()
    defer tx.Rollback()

    lockConsumerOffset(tx, "exec.fills")
    ensureNextOrReplayGap(fill.GlobalSeq)
    insertAppliedFill(tx, fill.FillId, fill.Ticker, fill.GlobalSeq)

    updatePosition(tx, fill.MakerUserId, fill.Ticker, signedQty(fill.MakerSide, fill.MakerAction, fill.Count), fill.PriceTicks, fill.GlobalSeq)
    updatePosition(tx, fill.TakerUserId, fill.Ticker, signedQty(fill.TakerSide, fill.TakerAction, fill.Count), fill.PriceTicks, fill.GlobalSeq)
    updateOpenInterest(tx, fill.Ticker, fill)
    advanceOffset(tx, "exec.fills", fill.GlobalSeq)

    tx.Commit()
}
```

### 4.3 Position math

For binaries, positions are normalized to a single signed YES axis:
- `BUY YES` or `SELL NO` => `+count` (long YES)
- `SELL YES` or `BUY NO` => `-count` (long NO / short YES)

For scalars:
- `BUY LONG` or `SELL SHORT` => `+count`
- `SELL LONG` or `BUY SHORT` => `-count`

Avg cost: weighted average. On reduction (qty change crosses zero or moves toward zero):
- Realized PnL increases by `|reduce_qty| × (current_price - avg_cost)` for longs reducing, etc.

This is straightforward but error-prone; it has a dedicated test suite.

### 4.4 Demo vs production

Same state machine and math in demo and production. The source transport changes from NATS Core + replay fallback to JetStream durable consumption, but `global_seq` ordering and `applied_fills` idempotency do not change.

---

## 5. `refdata-svc`

### 5.1 Responsibility

Authoritative metadata for contracts, events, series. Lifecycle state machine.

### 5.2 Lifecycle FSM

```
       admin           admin/scheduler    close_at         oracle finalized       settlement done
DRAFT ------> LISTED ----------------> OPEN ------> CLOSED --------------> RESOLVING ---------> SETTLED
                                          |             ^
                                          |             |
                                          v             |
                                       HALTED ----------+
                                          |
                                          v
                                      CANCELLED

DRAFT/LISTED/OPEN/HALTED/CLOSED may transition to CANCELLED through admin cancel.
```

Permitted transitions:

```go
var permittedTransitions = map[ContractState][]ContractState{
    DRAFT:     {LISTED, CANCELLED},
    LISTED:    {OPEN, CANCELLED},
    OPEN:      {HALTED, CLOSED, CANCELLED},
    HALTED:    {OPEN, CLOSED, CANCELLED},
    CLOSED:    {RESOLVING, CANCELLED},
    RESOLVING: {SETTLED, CANCELLED},
    SETTLED:   {},
    CANCELLED: {},
}
```

`OPEN -> CLOSED` is a coordinated transition:
1. `refdata-svc` marks the contract as closing and rejects new open transitions.
2. `admin-svc`/scheduler calls `me-core.CloseBook(ticker)`.
3. `me-core` stops accepting new submits for the ticker, drains earlier commands, expires all resting orders with reason `BOOK_CLOSED`, and returns `close_global_seq`.
4. `order-router` persists all terminal order events through `close_global_seq` and releases holds for unfilled resting quantity.
5. `refdata-svc` persists `contracts.close_global_seq` and emits `md.lifecycle.<ticker>`.
6. Settlement is allowed to start only after orders, hold releases, ledger posting, and position consumers have caught up through that sequence.

### 5.3 Demo behavior

A background goroutine ("contract scheduler") checks every 5 seconds:
- Contracts where `state == LISTED AND open_at <= now()`: transition to `OPEN`.
- Contracts where `state == OPEN AND close_at <= now()`: transition to `CLOSED`.

In demo we manually override times to "in 30 seconds" so investors can watch the full lifecycle.

### 5.4 Caching

`refdata-svc` is hit on every order submission. We cache aggressively in callers (`order-router`, `risk-svc`, `me-core`) with a 60-second TTL and invalidate via a NATS broadcast (`refdata.contract_updated`). State transitions that close or halt trading must bypass stale cache by forcing a refdata refresh before the next order validation.

---

## 6. `audit-svc`

### 6.1 Responsibility

Consume `audit.events` from NATS, write to Postgres `audit.events`. Production: also stream to S3 as Parquet and maintain hash chain.

### 6.2 Demo implementation (single consumer)

```go
sub, _ := nats.Subscribe("audit.events", func(msg *nats.Msg) {
    var ev AuditEvent
    proto.Unmarshal(msg.Data, &ev)
    db.Exec(`INSERT INTO audit.events
             (event_seq, service, type, actor, subject, payload, trace_id, ts)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)`,
        ev.EventSeq, ev.Service, ev.Type, ev.Actor, ev.Subject,
        ev.Payload, ev.TraceId, ev.Ts.AsTime())
})
```

### 6.3 Event sources (who publishes)

- `order-router`: ORDER_SUBMITTED, ORDER_PENDING, ORDER_ACCEPTED, ORDER_CANCELLED, ORDER_AMENDED, ORDER_REJECTED, ORDER_FILLED, FILL_LEDGER_POSTED
- `ledger-svc`: HOLD_PLACED, HOLD_RELEASED, HOLD_COMMITTED, DEPOSIT_CREDITED, WITHDRAWAL_REQUESTED, FEE_CHARGED, SETTLEMENT_POSTED, LEDGER_EVENT_PUBLISHED
- `refdata-svc`: CONTRACT_CREATED, CONTRACT_STATE_TRANSITION
- `oracle-svc`: ATTESTATION_RECEIVED, RESOLUTION_PROPOSED, RESOLUTION_FINALIZED
- `settlement-svc`: SETTLEMENT_STARTED, SETTLEMENT_BLOCKED_WAITING_FOR_FILLS, SETTLEMENT_PAYOUT_POSTED, SETTLEMENT_COMPLETED
- `admin-svc`: every admin RPC call (ADMIN_USER_FROZEN, ADMIN_CONTRACT_CANCELLED, etc.)
- `gw-rest`: LOGIN_SUCCESS, LOGIN_FAILURE, MFA_CHALLENGE (production)

A small Go helper `pkg/audit/emitter.go` is used by every service:

```go
audit.Emit(ctx, "ORDER_SUBMITTED", actorUserID, orderID, map[string]any{
    "ticker": ticker,
    "side": side,
    "count": count,
    "price_ticks": priceTicks,
})
```

This handles seq generation (via a Postgres sequence consulted by audit-svc — or a NATS-backed counter in production for HA), hash chaining (production), and NATS publish.

---

## 7. `admin-svc`

### 7.1 Responsibility

Internal-only API for ops actions. Lives behind VPN in production. In demo, runs as a separate service exposed only via the admin frontend at `localhost:3001`.

### 7.2 Endpoints (gRPC)

```
service Admin {
  // Contracts
  rpc CreateContract(...) -- proxies to refdata
  rpc TransitionContractState(...) -- proxies to refdata, with audit
  rpc CancelContract(...) -- cascades to me-core + settlement (refund all)

  // Accounts
  rpc CreditFakeDeposit(...) -- proxies to ledger.AdminCreditDeposit
  rpc FreezeUser(user_id) -- updates users.status = 'FROZEN'
  rpc UnfreezeUser(user_id)
  rpc SetUserLimits(user_id, limits)

  // Oracle
  rpc ForceResolution(event_ticker, value)

  // Operations
  rpc ResetDemo() -- ONLY enabled when env DEMO_MODE=true; clears all state

  // Read
  rpc GetSystemHealth() -- aggregates health from all services
  rpc StreamAuditLog() -- live tail
}
```

Every admin RPC requires dual approval in production (two distinct admin tokens). Demo: single admin user.

---

## 8. Service Health and Liveness

Every service exposes:

- `GET /health/live` — process is alive (200 if up)
- `GET /health/ready` — service can take traffic (200 if dependencies healthy)
- `GET /metrics` — Prometheus metrics

Health checks include downstream connectivity:
- `gw-rest` ready iff order-router gRPC reachable + Redis pingable
- `order-router` ready iff ME + risk + ledger + refdata reachable, Postgres pingable
- `ledger-svc` ready iff Postgres pingable + NATS reachable
- `me-core` ready iff books restored + no crossed restored books + NATS reachable + Postgres pingable (for demo restore)

Demo runs these but doesn't act on them. Production uses them for K8s readiness gates and ALB target groups.

---

## 9. Shared Libraries (`pkg/`)

```
pkg/
├── auth/          # JWT decode, mTLS helpers (production)
├── idem/          # idempotency cache abstractions (in-mem + Redis impls)
├── tracing/       # OpenTelemetry init + helpers
├── pgx/           # Postgres helpers, migrations, retry-on-conflict
├── natsutil/      # NATS connection mgmt, JSON+proto codec
├── snowflake/     # Snowflake ID generator
├── audit/         # Audit event emitter
├── refclient/     # Cached refdata gRPC client
└── proto/         # Generated proto bindings (auto-generated by scripts/proto-gen.sh)
```

These are imported by every service. Changes get reviewed by the platform owner before merge.
