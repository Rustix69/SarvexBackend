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
   e. Publish NATS subject oracle.resolutions.finalized.<event_ticker>
4. settlement-svc consumes oracle.resolutions.finalized, kicks off settlement (see §2)
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
6. After window expires with no dispute: oracle-svc auto-FinalizeResolution -> publishes oracle.resolutions.finalized
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
    var finalized Resolution
    proto.Unmarshal(msg.Data, &finalized)
    go settleEvent(ctx, finalized.EventTicker)
})

func settleEvent(ctx, eventTicker string) {
    // 1. Get finalized resolution from oracle-svc.
    res, _ := oracleClient.GetResolution(ctx, &GetResolutionRequest{EventTicker: eventTicker})
    if res.Status != RESOLUTION_FINALIZED { return }

    // 2. Find all contracts referencing this event.
    contracts, _ := refdataClient.ListContracts(ctx, &ListContractsRequest{})
    for _, c := range contracts {
        if c.EventTicker != eventTicker { continue }
        if c.State != CLOSED && c.State != RESOLVING { continue }
        settleContract(ctx, c, res)
    }
}
```

Settlement never starts from a live/open book. Each contract must have `close_global_seq` set by `me-core.CloseBook` and persisted by `refdata-svc` during `OPEN -> CLOSED`.

### 2.3 Settling a single contract

```go
func settleContract(ctx, contract Contract, res Resolution) {
    // 0. Serialize by ticker. If another worker owns this ticker, exit.
    settlement := lockOrCreateSettlementRow(contract.Ticker)
    if settlement.Status == COMPLETE { return }

    closeSeq := contract.CloseGlobalSeq
    if closeSeq == 0 { fail("missing close_global_seq") }

    // 1. Block until order/fill/ledger/position paths have caught up to closeSeq.
    waitUntilOrderRouterHasPersistedFillsAndTerminalOrders(contract.Ticker, closeSeq)
    waitUntilClosedOrderHoldsReleased(contract.Ticker, closeSeq)
    waitUntilAllFillsLedgerPosted(contract.Ticker, closeSeq)
    waitUntilPositionSvcApplied(contract.Ticker, closeSeq)

    // 2. Transition state CLOSED -> RESOLVING if needed.
    refdataClient.TransitionState(ctx, &TransitionStateRequest{
        Ticker: contract.Ticker, NewState: RESOLVING,
    })

    // 3. Compute payout per contract from settlement_rule and oracle value.
    payoutPerContract := payoutPerContract(contract, res)

    // 4. Persist settlement intent with close_global_seq.
    upsertSettlementInProgress(contract, res, payoutPerContract, closeSeq)

    // 5. Fetch positions by contract after position-svc confirms min_global_seq.
    positions, _ := positionClient.ListPositionsByContract(ctx, &ListPositionsByContractRequest{
        Ticker: contract.Ticker, IncludeClosed: false, MinGlobalSeq: closeSeq,
    })

    // 6. For each position, persist payout intent and post ledger idempotently.
    for _, p := range positions {
        if p.NetQty == 0 { continue }
        payout := computeUserPayout(p, payoutPerContract, contract)
        idemKey := fmt.Sprintf("settlement:%s:%s", contract.Ticker, p.UserId)

        insertPayoutIntent(contract.Ticker, p, payout, idemKey)
        if payout > 0 {
            txID := ledgerClient.PostTransaction(ctx, &PostTransactionRequest{
                IdempotencyKey: idemKey,
                ReasonCode: "SETTLEMENT",
                Entries: []LedgerEntry{
                    {AccountCode: "LIAB:HOUSE:UNSETTLED_TRADES:" + contract.Ticker,
                     Direction: "DR", AmountMicroUsdc: payout},
                    {AccountCode: "LIAB:USER:" + p.UserId + ":CASH",
                     Direction: "CR", AmountMicroUsdc: payout},
                },
            })
            markPayoutPosted(idemKey, txID)
        }
    }

    // 7. Sweep residual rounding only after all payouts are POSTED.
    roundingTxID := sweepRounding(ctx, contract.Ticker)

    // 8. Mark settlement complete, then transition RESOLVING -> SETTLED.
    markSettlementComplete(contract.Ticker, roundingTxID)
    refdataClient.TransitionState(ctx, &TransitionStateRequest{
        Ticker: contract.Ticker, NewState: SETTLED,
    })

    audit.Emit(ctx, "SETTLEMENT_COMPLETED", "service:settlement-svc", contract.Ticker, ...)
}
```

If any prerequisite is missing, settlement remains `PENDING` or `IN_PROGRESS` and emits `SETTLEMENT_BLOCKED_WAITING_FOR_FILLS`. It must not mark the contract `SETTLED` until every payout row is `POSTED`, the rounding sweep has posted, and the unsettled-trades invariant passes.

### 2.4 Payout formulas

**Binary:** the oracle reports an event outcome, not necessarily `YES`/`NO`. The contract's `settlement_rule` maps that outcome to the YES payout.

Example rule for `RBI-JUN26-CUT25`:
```json
{"type":"categorical_equals","yes_values":["CUT_25","CUT_50","CUT_75","CUT_100"]}
```

```go
func binaryPayout(c Contract, r Resolution) (yesPayout, noPayout int64) {
    yesWins := settlementRuleMatches(c.SettlementRule, r.CategoricalValue, r.NumericValue)
    if yesWins { return 1000000, 0 }
    return 0, 1000000
}

// Positive net_qty = long YES. Negative net_qty = long NO.
```

**Scalar:** use integer arithmetic only. No floats in settlement.

```go
func scalarPayout(c Contract, r Resolution) int64 {
    realized := clamp(r.NumericValue, c.LowerBoundTicks, c.UpperBoundTicks)
    numerator := (realized - c.LowerBoundTicks) * c.MultiplierMicroUsdc
    denominator := c.UpperBoundTicks - c.LowerBoundTicks
    return numerator / denominator // round down to micro_usdc; residual swept later
}

// Positive net_qty = long; negative net_qty = short.
// short payout = multiplier_micro_usdc - scalarPayout.
```

Rounding: integer division may create residual micro_usdc in `LIAB:HOUSE:UNSETTLED_TRADES:<ticker>`. After all user payouts are posted, sweep residual to `REVENUE:SETTLEMENT_ROUNDING` and store `rounding_sweep_tx_id`.

### 2.5 Idempotency and crash safety

Every payout has idempotency key `settlement:<ticker>:<user_id>`. The settlement row is locked per ticker, so only one worker can settle a contract at a time.

Crash safety rules:
1. If the worker crashes before payout intents are inserted, restart re-enters from the settlement row and recomputes against the same `close_global_seq`.
2. If it crashes after some payout intents are inserted, restart resumes rows where `status != POSTED`.
3. If it crashes after a ledger post but before marking the payout row posted, retrying `Ledger.PostTransaction` with the same idempotency key returns the same transaction.
4. If `position-svc` lags, settlement waits for `ListPositionsByContract(min_global_seq=close_global_seq)`. If unavailable, it replays fills through `OrderRouter.ListFills` rather than reading another service's tables directly.
5. Settlement cannot transition to `SETTLED` until all fills up to `close_global_seq` have `ledger_post_status='POSTED'`.

### 2.6 The `LIAB:HOUSE:UNSETTLED_TRADES:<ticker>` account

When orders fill during normal trading, committed maker/taker holds become credits to `UNSETTLED_TRADES:<ticker>`. At settlement, this account holds the total committed capital for the contract. Payouts debit from it. **After all payouts**, residual rounding is swept to REVENUE.

Invariant: after settlement is `COMPLETE`, `UNSETTLED_TRADES:<ticker>` balance must be 0 unless the documented rounding sweep leaves a bounded sub-contract dust amount before the sweep transaction. Anything larger is a P0 correctness issue.

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
      Balance display: settlement payout credited +$100; the earlier $62 trade cost already moved from hold to unsettled-trades escrow at fill time

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
- Position-svc lag scenario: settle while position-svc behind; verify wait for `close_global_seq` and replay through `OrderRouter.ListFills`.

---

## 6. What this design avoids

- **No DAO / token / on-chain governance.** Oracle disputes resolve via admin escalation. Phase 4 could move to UMA-style optimistic oracle, but we don't need it.
- **No auto-rebalancing of fee accounts.** Fees accumulate in REVENUE accounts; ops manually withdraws to operating accounts on schedule.
- **No "contract void" logic during a dispute.** If resolution is disputed and admin can't determine outcome, contract goes to CANCELLED state and all positions are refunded their original cost basis. (Demo skips this path.)
