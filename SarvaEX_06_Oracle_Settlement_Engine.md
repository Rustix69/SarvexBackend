# 06 — Oracle & Settlement

These two services together turn a real-world outcome into ledger entries crediting winners.

---

## 1. `oracle-svc`

### 1.1 Responsibility

Ingest event outcome data, attest to it, and finalize resolutions. Phase 1 supports three oracle policies; demo only uses one (admin-driven).

### 1.2 Oracle policies

| Policy | Behavior | Demo support? |
|---|---|---|
| `ADMIN` | Admin manually calls `ForceResolution` with dual approval. | **Yes — used in demo.** |
| `SINGLE_SOURCE` | One trusted attestor source (e.g., MoSPI URL scraper). | Phase 2 |
| `MULTI_SOURCE_ATTEST` | K-of-N signed attestations within a challenge window. | Phase 2 |

Policy is per-contract, declared in `refdata.contracts.oracle_policy`.

### 1.3 Demo flow

```
1. Admin clicks "Resolve event RBI-JUN26 → CUT_25" in admin console
2. admin-svc → oracle-svc.AdminForceResolution(event_ticker, value, admin_id, justification)
3. oracle-svc:
   a. Verify admin role and dual approval (in demo: single approval allowed, audit-logged)
   b. INSERT INTO oracle.resolutions (event_ticker, status='FINALIZED', numeric_value, categorical_value, finalized_at)
   c. INSERT INTO oracle.attestations (event_ticker, attestor_id='admin:<admin_id>', ...)
   d. Emit audit event RESOLUTION_FINALIZED
   e. Publish NATS subject resolution.finalized.<event_ticker>
4. settlement-svc consumes resolution.finalized, kicks off settlement (see §2)
```

### 1.4 Production flow (`MULTI_SOURCE_ATTEST`)

```
1. Background scheduler in oracle-svc polls each contract approaching its expected_resolution_at
2. For each, fans out to attestor workers (separate processes/pods running as different K8s service accounts)
3. Each attestor:
   a. Fetches its assigned data source (e.g., RBI press release page, CME settlement feed)
   b. Parses the value
   c. Signs (Ed25519) the canonical message: SHA256(event_ticker || value || source || observed_at)
   d. Sends ProposeResolution to oracle-svc
4. oracle-svc:
   a. INSERTs the attestation, verifying the signature against oracle_keys.public_key
   b. Updates oracle.resolutions.attestor_count
   c. On reaching required_quorum: sets status='PROPOSED', challenge_window_ends_at=now()+30min
5. Challenge window: any registered attestor can submit a DISPUTING attestation
6. After window expires with no dispute: oracle-svc auto-FinalizeResolution → publishes resolution.finalized
7. If disputed: status='DISPUTED'; falls back to ADMIN policy (admin manually resolves with extra scrutiny)
```

### 1.5 Schema relationship

```
oracle.resolutions       (one row per event_ticker)
oracle.attestations      (many rows per event_ticker)
oracle.oracle_keys       (one row per attestor_id)
refdata.events           (event_ticker → expected_resolution_at, source, policy)
refdata.contracts        (contracts referencing events)
```

### 1.6 NATS subjects

- `oracle.attestations.<event_ticker>` — every new attestation (used for monitoring)
- `oracle.resolutions.proposed.<event_ticker>` — proposal made (used for challenge window UI)
- `oracle.resolutions.finalized.<event_ticker>` — finalized, settlement starts
- `oracle.resolutions.disputed.<event_ticker>` — dispute filed

---

## 2. `settlement-svc`

### 2.1 Responsibility

When a resolution finalizes, settle every open position in every contract referencing that event. Translate outcome → payout per position → ledger transactions crediting winners.

### 2.2 Trigger

```go
sub, _ := nats.Subscribe("oracle.resolutions.finalized.*", func(msg *nats.Msg) {
    var finalized ResolutionFinalizedEvent
    proto.Unmarshal(msg.Data, &finalized)
    go settleEvent(ctx, finalized.EventTicker)
})

func settleEvent(ctx, eventTicker string) {
    // 1. Get the resolution
    res, _ := oracleClient.GetResolution(ctx, &GetResolutionRequest{EventTicker: eventTicker})

    // 2. Find all contracts referencing this event
    contracts, _ := refdataClient.ListContracts(ctx, &ListContractsRequest{}) // filter by event_ticker
    for _, c := range contracts {
        if c.EventTicker != eventTicker { continue }
        if c.State != ContractState_CLOSED && c.State != ContractState_RESOLVING { continue }
        settleContract(ctx, c, res)
    }
}
```

### 2.3 Settling a single contract

```go
func settleContract(ctx, contract Contract, res Resolution) {
    // Transition state CLOSED → RESOLVING
    refdataClient.TransitionState(ctx, &TransitionStateRequest{
        Ticker: contract.Ticker, NewState: RESOLVING,
    })

    // Compute winner payout per contract
    var payoutPerContract int64
    switch contract.Kind {
    case BINARY:
        payoutPerContract = binaryPayout(contract, res)
    case SCALAR:
        payoutPerContract = scalarPayout(contract, res)
    }

    // Record settlement intent
    db.Exec(`INSERT INTO settlement.settlements
        (ticker, event_ticker, numeric_value, categorical_value,
         winner_payout_per_contract_micro_usdc, status, started_at)
        VALUES ($1, $2, $3, $4, $5, 'IN_PROGRESS', now())
        ON CONFLICT (ticker) DO NOTHING`, ...)

    // Fetch all open positions in this contract
    positions, _ := positionClient.ListAllPositionsForContract(ctx, contract.Ticker)

    // For each position, post a ledger transaction
    for _, p := range positions {
        if p.NetQty == 0 { continue }
        payout := computeUserPayout(p, payoutPerContract, contract)
        idemKey := fmt.Sprintf("settlement:%s:%s", contract.Ticker, p.UserId)

        // Persist intent
        db.Exec(`INSERT INTO settlement.settlement_payouts
            (ticker, user_id, position_qty, payout_micro_usdc, idempotency_key)
            VALUES ($1, $2, $3, $4, $5) ON CONFLICT (idempotency_key) DO NOTHING`,
            contract.Ticker, p.UserId, p.NetQty, payout, idemKey)

        // Post ledger transaction
        if payout > 0 {
            ledgerClient.PostTransaction(ctx, &PostTransactionRequest{
                IdempotencyKey: idemKey,
                ReasonCode: "SETTLEMENT",
                Entries: []LedgerEntry{
                    {AccountCode: "LIAB:HOUSE:UNSETTLED_TRADES:" + contract.Ticker,
                     Direction: "DR", AmountMicroUsdc: payout},
                    {AccountCode: "LIAB:USER:" + p.UserId + ":CASH",
                     Direction: "CR", AmountMicroUsdc: payout},
                },
            })
        }
    }

    // After all payouts: any residual in UNSETTLED_TRADES house account
    // due to rounding is moved to REVENUE:SETTLEMENT_ROUNDING.
    sweepRounding(ctx, contract.Ticker)

    // Mark contract settled
    refdataClient.TransitionState(ctx, &TransitionStateRequest{
        Ticker: contract.Ticker, NewState: SETTLED,
    })

    // Update settlement row
    db.Exec(`UPDATE settlement.settlements SET status='COMPLETE', completed_at=now() WHERE ticker=$1`,
        contract.Ticker)

    audit.Emit(ctx, "SETTLEMENT_COMPLETED", "service:settlement-svc", contract.Ticker, ...)
}
```

### 2.4 Payout formulas

**Binary (YES side wins):**
```go
func binaryPayout(c Contract, r Resolution) (yesPayout, noPayout int64) {
    // Demo binary: r.CategoricalValue is the "winning side" e.g., "YES" or "NO"
    if r.CategoricalValue == "YES" {
        return 1000000, 0   // $1.00 to YES holders, $0 to NO
    }
    return 0, 1000000
}

// For a user holding net_qty contracts:
//   positive net_qty in YES contract = "long YES"  → payout = qty * yesPayout
//   negative net_qty in YES contract = "short YES" = "long NO" → payout = |qty| * noPayout
// (We represent both YES and NO as one bookkeeping side; "NO contract" is just short YES.)
```

**Scalar:**
```go
func scalarPayout(c Contract, r Resolution) int64 {
    realized := r.NumericValue                              // already in tick units
    if realized < c.LowerBoundTicks { realized = c.LowerBoundTicks }
    if realized > c.UpperBoundTicks { realized = c.UpperBoundTicks }
    rangeTicks := c.UpperBoundTicks - c.LowerBoundTicks
    fraction := float64(realized - c.LowerBoundTicks) / float64(rangeTicks)
    return int64(fraction * float64(c.MultiplierMicroUsdc))
}

// For a user holding net_qty contracts (signed):
//   positive = long  → payout = qty * scalarPayout
//   negative = short → payout = |qty| * (multiplier - scalarPayout)
```

Rounding: any sub-micro_usdc remainder is dropped (we work in integer micro_usdc); the small residuals are swept to `REVENUE:SETTLEMENT_ROUNDING` at the end.

### 2.5 Idempotency and crash safety

Every payout has an idempotency key: `settlement:<ticker>:<user_id>`. If the settlement worker crashes mid-way:
1. On restart, it sees the settlement row exists with `status=IN_PROGRESS`.
2. It re-queries positions and replays. Each ledger.PostTransaction with the same idempotency key is a no-op.
3. Eventually all payouts post; status transitions to COMPLETE.

If `position-svc` is lagging when settlement runs (a real risk: settlement may fire moments after the last trade), settlement waits up to 30 seconds for `position-svc` to catch up to the close_at trade seq. If still lagging, settlement reads directly from `orders.fills` to reconstruct positions (a fallback path).

### 2.6 The `LIAB:HOUSE:UNSETTLED_TRADES:<ticker>` account

When orders fill during normal trading, the buyer's hold becomes a CR to `UNSETTLED_TRADES:<ticker>`. At settlement, this account holds the total committed capital for the contract. Payouts DR from it. **After all payouts**, residual rounding stays in this account; we sweep it to REVENUE.

Invariant: after settlement is `COMPLETE`, `UNSETTLED_TRADES:<ticker>` balance should be ≤ a few micro_usdc (rounding dust). If it's more, something's wrong — alert.

---

## 3. Demo settlement walkthrough (used in investor demo)

This is the script for the live demo's settlement scene:

```
T=0   "Now I'll resolve the RBI June MPC event. In production, this happens via signed attestations from independent oracle operators. For the demo, I'll trigger it from the admin console."

T=5   Admin clicks "Resolve" → selects "CUT_25" → confirms

T=6   Admin console shows: "Resolution finalized at 14:32:01.123. Settlement starting..."

T=7   Trading UI: contract state badge changes OPEN → CLOSED → RESOLVING → SETTLED in real-time
      (each transition shows as a flash on the contract card)

T=8   The retail user's blotter: open position +100 YES turns into "Settled +$100 USDC"
      Balance display: +$62 → +$100 (the $62 hold released, $100 payout credited)

T=10  Switch to ops view: "Total settled: $1,247.00 across 8 positions. Settlement time: 1.4s."
      (numbers are real, from the live ledger)

T=15  "And there's an audit trail of every payout."
      Open audit log filtered to the settlement events: 8 SETTLEMENT_POSTED entries,
      1 RESOLUTION_FINALIZED, 1 SETTLEMENT_COMPLETED.
```

This entire scene is real code, real ledger operations, real position calculations. Nothing is mocked except the oracle's choice of policy (admin instead of multi-source).

---

## 4. File layouts

```
services/oracle-svc/
├── cmd/server/main.go
├── internal/
│   ├── server/        # gRPC: ProposeResolution, FinalizeResolution, AdminForceResolution
│   ├── attestation/   # signature verify (production)
│   ├── scheduler/     # poll-and-attest (production)
│   ├── persistence/
│   └── nats/
└── go.mod

services/settlement-svc/
├── cmd/server/main.go
├── internal/
│   ├── server/        # gRPC: SettleContract, GetSettlement
│   ├── worker/        # NATS consumer + settle loop
│   ├── payouts/       # binary/scalar formulas
│   ├── persistence/
│   └── ledgerclient/
└── go.mod
```

---

## 5. Test cases (settlement)

### Unit
- Binary YES wins: 3 users with various positions get correct payouts.
- Binary NO wins: same with NO winning.
- Scalar at lower bound: long gets 0, short gets full multiplier.
- Scalar at upper bound: long gets full, short gets 0.
- Scalar at midpoint: long and short get equal halves.
- Scalar with rounding: dust swept to REVENUE:SETTLEMENT_ROUNDING.
- Idempotency: replay same settlement; all transactions are no-ops.

### Integration
- Trade 100 contracts at various prices, resolve YES, verify house UNSETTLED_TRADES drains to 0 ± dust.
- Crash settlement worker mid-payout; restart; verify completion.
- Position-svc lag scenario: settle while position-svc behind; verify wait-or-fallback works.

---

## 6. What this design avoids

- **No DAO / token / on-chain governance.** Oracle disputes resolve via admin escalation. Phase 4 could move to UMA-style optimistic oracle, but we don't need it.
- **No auto-rebalancing of fee accounts.** Fees accumulate in REVENUE accounts; ops manually withdraws to operating accounts on schedule.
- **No "contract void" logic during a dispute.** If resolution is disputed and admin can't determine outcome, contract goes to CANCELLED state and all positions are refunded their original cost basis. (Demo skips this path.)
