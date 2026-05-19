# 05 — Core Services

Detailed designs for: `order-router`, `risk-svc`, `ledger-svc`, `position-svc`, `refdata-svc`, `audit-svc`, `admin-svc`.

Each section follows the same structure: responsibility, demo implementation, production implementation, key algorithms, failure modes.

---

## 1. `order-router`

### 1.1 Responsibility

Orchestrate the order entry pipeline: validate → risk check → place hold → submit to ME → respond. Persist orders and fills. The single synchronous flow that ties everything together.

### 1.2 Inbound flow

```
gw-rest → order-router.SubmitOrder()
  1. Validate request shape (proto validation)
  2. Look up contract spec from refdata (Redis cache, 60s TTL)
  3. Validate: tick alignment, price range, qty, contract state == OPEN
  4. Generate order_id (Snowflake)
  5. INSERT INTO orders.orders (status='PENDING') -- becomes the durable record
  6. Risk.PreTradeCheck() → {approved, required_hold_micro_usdc}
     - On reject: UPDATE orders SET status='REJECTED'; return
  7. Ledger.PlaceHold(idempotency_key=order_id, amount=required_hold) → {hold_id}
     - On fail: UPDATE orders SET status='REJECTED' reason='INSUFFICIENT_FUNDS'; return
  8. MatchingEngine.SubmitOrder(order_id, hold_id, ticker, side, action, price, qty, flags, stp)
     - On timeout: Ledger.ReleaseHold(hold_id); UPDATE orders SET status='REJECTED' reason='ME_TIMEOUT'; return 503
     - On reject: Ledger.ReleaseHold(hold_id); UPDATE orders SET status='REJECTED' reason=<code>
     - On accept: UPDATE orders SET status='OPEN' (or PARTIAL/FILLED if immediate fills)
  9. For each immediate fill: persist to orders.fills; emit ledger fill transaction (async, see §1.5)
  10. Return ack with order, fills
```

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
| Request-level | `Idempotency-Key` header | 24h | Redis |
| Client-order-level | `(user_id, client_order_id)` | forever | Postgres UNIQUE constraint |
| ME-level | `order_id` | session | me-core's in-memory LRU |

If a client retries `POST /v1/orders` with the same `Idempotency-Key`, we return the previously computed response from Redis. If the client retries with a different `Idempotency-Key` but the same `client_order_id`, the Postgres UNIQUE constraint catches it; we look up the prior order_id and return the order in its current state.

### 1.5 Post-fill ledger work

When the ME returns fills (synchronously), we need to:
1. Persist fills to `orders.fills`.
2. Update `orders.orders` filled/remaining counters.
3. Commit the proportional hold and post the trade ledger transaction.

Steps 1-2 are in the same Postgres transaction with the order update. Step 3 is **async** because it requires another service call and we want to ack the client first:

```go
func handleFills(ctx, orderID, fills) {
    tx := db.Begin()
    for _, f := range fills {
        tx.Exec(`INSERT INTO orders.fills (...)`, ...)
    }
    tx.Exec(`UPDATE orders.orders SET filled_count=$1, remaining_count=$2, status=$3 WHERE order_id=$4`,
        totalFilled, remaining, newStatus, orderID)
    tx.Commit()

    // Async: post ledger transactions for each fill
    for _, f := range fills {
        ledgerTxQueue.Push(ledgerTxJob{
            idempotencyKey: "fill:" + f.FillID,
            entries: makeFillEntries(f),
        })
    }
}
```

The `ledgerTxQueue` is a Go channel feeding a worker pool. If a ledger post fails, we retry with exponential backoff (idempotency key prevents duplicates). After 10 retries, we alert and stop the order book for that contract — money has moved on the book but not on the ledger, which is a P0 incident.

In production we replace this with NATS JetStream: ME publishes fills to `exec.fills.<ticker>`; a dedicated `fill-poster` worker subscribes and posts to ledger with at-least-once + idempotency.

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

`risk.working_orders_summary` is maintained by subscribing to `exec.*` events:
- On `OrderAccepted`: increment `(user, ticker, side).total_qty` and `.total_max_loss_micro_usdc`.
- On `Fill`: decrement filled qty proportionally.
- On `Cancel`: decrement cancelled qty.

This is **eventually consistent**. Risk uses it for position projection. The tradeoff: a user could submit a flurry of orders faster than risk can update the summary, briefly exceeding the limit. In demo we accept this. In production, we add a Redis Lua script that atomically reads-and-increments the summary on each PreTradeCheck call.

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
│   ├── consumer/      # NATS consumer for exec.* events
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
    // 1. Validate balanced
    if err := assertBalanced(req.Entries); err != nil {
        return nil, status.Errorf(InvalidArgument, "unbalanced: %v", err)
    }

    // 2. Idempotency check
    var existingTxID int64
    err := db.QueryRow(`SELECT tx_id FROM ledger.transactions WHERE idempotency_key=$1`,
        req.IdempotencyKey).Scan(&existingTxID)
    if err == nil {
        return &PostTransactionResponse{TxId: strconv.FormatInt(existingTxID, 10)}, nil
    }

    // 3. Single Postgres transaction
    tx, err := db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
    if err != nil { return nil, err }
    defer tx.Rollback()

    // 4. Insert transaction row
    var txID int64
    err = tx.QueryRow(`INSERT INTO ledger.transactions (idempotency_key, reason_code, metadata)
                       VALUES ($1, $2, $3) RETURNING tx_id`,
        req.IdempotencyKey, req.ReasonCode, req.Metadata).Scan(&txID)
    if err != nil {
        if isDuplicateKeyErr(err) {
            // Concurrent caller won; look up and return
            ...
        }
        return nil, err
    }

    // 5. Lock and resolve all accounts in deterministic order to avoid deadlock
    accountIDs := make([]int64, 0, len(req.Entries))
    accountCodes := uniqueSortedAccountCodes(req.Entries)
    for _, code := range accountCodes {
        accountID, err := ensureAccount(tx, code)  // lazy create
        if err != nil { return nil, err }
        accountIDs = append(accountIDs, accountID)
    }
    // Lock account rows in id-order
    sortAndLock(tx, accountIDs)

    // 6. For each entry: compute running balance and insert
    for _, e := range req.Entries {
        var prevBalance, prevSeq int64
        err := tx.QueryRow(`SELECT running_balance_micro_usdc, account_seq
                            FROM ledger.entries WHERE account_id=$1
                            ORDER BY account_seq DESC LIMIT 1`,
            accountIDFor(e), ).Scan(&prevBalance, &prevSeq)
        if err == sql.ErrNoRows { prevBalance = 0; prevSeq = 0 }

        newBalance := prevBalance + signedAmount(e)  // DR/CR sign convention by account_type
        _, err = tx.Exec(`INSERT INTO ledger.entries
            (tx_id, account_id, direction, amount_micro_usdc,
             running_balance_micro_usdc, account_seq, memo)
            VALUES ($1, $2, $3, $4, $5, $6, $7)`,
            txID, accountIDFor(e), e.Direction, e.AmountMicroUsdc,
            newBalance, prevSeq+1, e.Memo)
        if err != nil { return nil, err }
    }

    // 7. The deferred constraint trigger validates balance at commit
    if err := tx.Commit(); err != nil { return nil, err }

    // 8. Publish to NATS for downstream consumers (position-svc, audit-svc, gw-ws)
    publishLedgerEvent(ctx, txID, req)

    return &PostTransactionResponse{TxId: strconv.FormatInt(txID, 10)}, nil
}
```

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

A hold is a logical reservation on top of two ledger transactions (place + release/commit). The `ledger.holds` table tracks state:

```go
func (s *Server) PlaceHold(ctx, req) (*PlaceHoldResponse, error) {
    holdID := req.IdempotencyKey  // typically order_id

    // 1. Pre-flight: check user has sufficient cash balance
    cashBalance := s.getCashBalance(req.UserId)
    if cashBalance < req.AmountMicroUsdc {
        return nil, status.Errorf(FailedPrecondition, "INSUFFICIENT_FUNDS")
    }

    // 2. Post the transaction (cash → holds)
    txReq := &PostTransactionRequest{
        IdempotencyKey: "place_hold:" + holdID,
        ReasonCode: "HOLD_PLACE",
        Entries: []LedgerEntry{
            {AccountCode: "LIAB:USER:" + req.UserId + ":CASH", Direction: "DR", AmountMicroUsdc: req.AmountMicroUsdc},
            {AccountCode: "LIAB:USER:" + req.UserId + ":HOLDS", Direction: "CR", AmountMicroUsdc: req.AmountMicroUsdc},
        },
    }
    if _, err := s.postTransactionInternal(ctx, txReq); err != nil {
        return nil, err
    }

    // 3. Record the hold (idempotent)
    _, err := s.db.ExecContext(ctx, `INSERT INTO ledger.holds
        (hold_id, user_id, amount_micro_usdc, reason, status)
        VALUES ($1, $2, $3, $4, 'ACTIVE')
        ON CONFLICT (hold_id) DO NOTHING`,
        holdID, req.UserId, req.AmountMicroUsdc, req.Reason)
    if err != nil { return nil, err }

    return &PlaceHoldResponse{HoldId: holdID}, nil
}
```

`ReleaseHold` and `CommitHold` similarly post balanced transactions plus update the `holds` row.

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

Maintain user × contract positions. Fed exclusively from the trade stream.

### 4.2 Consumer loop

```go
sub, _ := nats.Subscribe("md.trade.*", func(msg *nats.Msg) {
    var trade TradeEvent
    proto.Unmarshal(msg.Data, &trade)

    // Idempotency: only apply if seq > last_trade_seq for this ticker
    // For each user involved (maker + taker), update positions
    applyTrade(ctx, trade)
})
```

But wait — `md.trade.*` doesn't carry user IDs (it's public market data). We need user-attributed fills. So position-svc subscribes to `exec.*` instead:

```go
sub, _ := nats.Subscribe("exec.fills", func(msg *nats.Msg) {
    var fill MeFill
    proto.Unmarshal(msg.Data, &fill)
    applyFill(ctx, fill)
})

func applyFill(ctx, fill MeFill) {
    tx, _ := db.Begin()
    defer tx.Rollback()

    // Maker side
    updatePosition(tx, fill.MakerUserId, fill.Ticker, makerSignedQty(fill), fill.PriceTicks, fill.GlobalSeq)
    // Taker side
    updatePosition(tx, fill.TakerUserId, fill.Ticker, takerSignedQty(fill), fill.PriceTicks, fill.GlobalSeq)
    // Open interest
    updateOpenInterest(tx, fill.Ticker, fill)

    tx.Commit()
}

func updatePosition(tx, userID, ticker, signedQtyDelta, fillPriceTicks, fillSeq) {
    // Read current
    var (curQty, curAvgCost, curRealizedPnL, lastSeq int64)
    tx.QueryRow(`SELECT net_qty, avg_cost_micro_usdc, realized_pnl_micro_usdc, last_trade_seq
                 FROM position.positions
                 WHERE user_id=$1 AND ticker=$2 FOR UPDATE`,
        userID, ticker).Scan(&curQty, &curAvgCost, &curRealizedPnL, &lastSeq)

    // Idempotency
    if fillSeq <= lastSeq { return }  // already applied

    newQty := curQty + signedQtyDelta
    // ... position math (FIFO or weighted-avg cost); realized PnL on reduction
    newAvgCost, deltaRealizedPnL := computePositionMath(curQty, curAvgCost, signedQtyDelta, fillPriceTicks)

    tx.Exec(`INSERT INTO position.positions (user_id, ticker, net_qty, avg_cost_micro_usdc, realized_pnl_micro_usdc, last_trade_seq)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (user_id, ticker) DO UPDATE SET
                net_qty = EXCLUDED.net_qty,
                avg_cost_micro_usdc = EXCLUDED.avg_cost_micro_usdc,
                realized_pnl_micro_usdc = EXCLUDED.realized_pnl_micro_usdc,
                last_trade_seq = EXCLUDED.last_trade_seq,
                updated_at = now()`,
        userID, ticker, newQty, newAvgCost, curRealizedPnL + deltaRealizedPnL, fillSeq)
}
```

### 4.3 Position math

For binaries: signed_qty positive = long YES; negative = long NO (= short YES).
For scalars: signed_qty positive = long; negative = short.

Avg cost: weighted average. On reduction (qty change crosses zero or moves toward zero):
- Realized PnL increases by `|reduce_qty| × (current_price - avg_cost)` for longs reducing, etc.

This is straightforward but error-prone; it has a dedicated test suite.

### 4.4 Demo vs production

Same. position-svc is the same code in demo and production.

---

## 5. `refdata-svc`

### 5.1 Responsibility

Authoritative metadata for contracts, events, series. Lifecycle state machine.

### 5.2 Lifecycle FSM

```
       admin           admin/scheduler    close_at         oracle finalized       settlement done
DRAFT ──────► LISTED ─────────────► OPEN ────────► CLOSED ────────────────► RESOLVING ──────────► SETTLED
                                          ▲                                                       ▲
                                          │ admin: halt                                           │
                                          ▼                                                       │
                                       (HALTED via state transition; demo skips this)             │
                                                                                                  │
                          admin: cancel (refund all)                                               │
DRAFT/LISTED/OPEN ──────────────────────────────────────────────────► CANCELLED ──────────────────┘
```

Implemented as a state machine with permitted transitions encoded:

```go
var permittedTransitions = map[ContractState][]ContractState{
    DRAFT:     {LISTED, CANCELLED},
    LISTED:    {OPEN, CANCELLED},
    OPEN:      {CLOSED, CANCELLED},
    CLOSED:    {RESOLVING, CANCELLED},
    RESOLVING: {SETTLED, CANCELLED},
    SETTLED:   {},  // terminal
    CANCELLED: {},  // terminal
}
```

Transitions trigger:
- `OPEN → CLOSED`: emit `md.lifecycle.<ticker>`; me-core stops accepting new orders for the ticker but doesn't drop the book yet.
- `CLOSED → RESOLVING`: oracle-svc takes over.
- `RESOLVING → SETTLED`: settlement-svc completes payouts.

### 5.3 Demo behavior

A background goroutine ("contract scheduler") checks every 5 seconds:
- Contracts where `state == LISTED AND open_at <= now()`: transition to `OPEN`.
- Contracts where `state == OPEN AND close_at <= now()`: transition to `CLOSED`.

In demo we manually override times to "in 30 seconds" so investors can watch the full lifecycle.

### 5.4 Caching

`refdata-svc` is hit on every order submission. We cache aggressively in callers (`order-router`, `risk-svc`, `me-core`) with a 60-second TTL and invalidate via a NATS broadcast (`refdata.contract_updated`).

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

- `order-router`: ORDER_SUBMITTED, ORDER_CANCELLED, ORDER_AMENDED, ORDER_REJECTED, ORDER_FILLED
- `ledger-svc`: HOLD_PLACED, HOLD_RELEASED, HOLD_COMMITTED, DEPOSIT_CREDITED, WITHDRAWAL_REQUESTED, FEE_CHARGED, SETTLEMENT_POSTED
- `refdata-svc`: CONTRACT_CREATED, CONTRACT_STATE_TRANSITION
- `oracle-svc`: ATTESTATION_RECEIVED, RESOLUTION_PROPOSED, RESOLUTION_FINALIZED
- `settlement-svc`: SETTLEMENT_STARTED, SETTLEMENT_COMPLETED
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
- `me-core` ready iff books restored + NATS reachable + Postgres pingable (for restore)

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
