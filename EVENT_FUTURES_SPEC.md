# Event Futures — Engine Specification (MVP)

**Status:** v1.0 — replaces current production formulas
**Scope:** Fully collateralized, banded (floor/cap) cash-settled futures on numerical events.
No leverage, no variation margin, no hedging/cross-margin (see §13 for later phases).

---

## 0. TL;DR for the developer

1. **All contracts are denominated and settled in USDC.** All backend money math is integers: prices in `ticks`, money in `micro-USDC` (1 USDC = 1,000,000), products in `i128`. Every `$` in this document means USDC.
2. **One scaling constant per contract**: `tick_value_micro` = micro-USDC per 1 raw tick. Computed once at listing. Nothing else scales prices. `divider` is display-only.
3. **Cash movement on every fill depends only on the fill price and the band** — never on the user's average entry:
   - Opening a **long** at `p` pays `(p − lower)`; closing a long at `p` receives `(p − lower)`.
   - Opening a **short** at `p` pays `(upper − p)`; closing a short at `p` receives `(upper − p)`.
   (each × qty × `tick_value_micro`)
4. **Settlement**: long receives `(final − lower)`, short receives `(upper − final)`, with `final` clamped to `[lower, upper]`.
5. **Market escrow invariant** (must hold after every event): `escrow == open_interest × (upper − lower) × tick_value_micro`.
6. P&L numbers (realized/unrealized/net) are **derived for display** from FIFO lots; they never drive cash.

---

## 1. Concepts

| Term | Meaning |
|---|---|
| Contract | One listed market, e.g. "CPI YoY Oct-2026, band 4.00–6.00%" |
| Band | `[lower, upper]` — the contract settles clamped to this range; max loss per side is bounded |
| Long | Profits when the final value is higher than entry |
| Short | Profits when the final value is lower than entry |
| Lock | Cash a position has put into escrow (its maximum possible loss) |
| Escrow | Per-contract pool holding all locks; pays out at close/settlement |

A fully collateralized contract is economically a **bounded linear payoff**: every contract has exactly one long and one short, and together they fund the full band. The exchange never takes risk and no margin calls exist.

---

## 2. Units and scaling (fixes the production unit bug)

### 2.0 Denomination: USDC

All contracts are denominated, margined and settled in **USDC**. Throughout this spec, every `$` amount means **USDC** (1 USDC ≡ `1_000_000` micro-USDC).

| Rule | Detail |
|---|---|
| Base unit | `micro-USDC` = 10⁻⁶ USDC. This matches USDC's 6 on-chain decimals on Ethereum, Solana, Base, Arbitrum, Polygon. |
| Chain decimals | **Normalize at the deposit/withdrawal boundary only.** Some bridged/pegged USDC variants use 18 decimals (e.g. Binance-Peg USDC on BNB Chain). Convert to micro-USDC on deposit; reject withdrawals that would require sub-micro precision. The engine never sees chain decimals. |
| Multiplier | Always stated as **USDC per 1 display unit** of the underlying, e.g. "10 USDC per €1/MWh", "10 USDC per bp", "0.01 USDC per $1 of BTC". |
| Non-USD underlyings (quanto) | Where the underlying is quoted in another currency (TTF in €, a JPY index), the multiplier is **fixed in USDC**. No FX conversion at any point: a €1 move pays exactly the stated USDC amount regardless of EUR/USD. Label these contracts "quanto (USDC-settled)" in the UI. |
| USD underlyings | BTC, US stocks, etc. are quoted in USD but paid in USDC 1:1 by definition. The engine does not adjust for any USDC/USD deviation. |
| Depeg disclosure | Users receive USDC tokens, not US dollars. Terms of use must state that USDC value vs USD is not guaranteed by the exchange. Escrow balances are held in USDC, so escrow is always fully funded in the settlement asset (no FX mismatch for the exchange). |
| Fees | Charged in micro-USDC (§7). |

### 2.1 Stored fields

| Field | Type | Example (TTF) | Notes |
|---|---|---|---|
| `divider` | int | `100` | Raw ticks per 1 display unit. **Display only.** |
| `tick_size` | int | `1` | Minimum price increment, in raw ticks. Validation only. |
| `lower_ticks` | int | `2000` | Band floor (€20.00) |
| `upper_ticks` | int | `8000` | Band cap (€80.00) |
| `multiplier_micro_per_display_unit` | int | `10_000_000` | 10 USDC per €1/MWh (quanto), in micro-USDC |
| `tick_value_micro` | int | `100_000` | **Derived**, stored, used by all math ($0.10/tick) |

### 2.2 The one scaling formula

```text
tick_value_micro = multiplier_micro_per_display_unit / divider
```

**Listing-time validation (reject the contract if any fails):**
- `multiplier_micro_per_display_unit % divider == 0` (no fractional micro-USDC, ever)
- `tick_value_micro > 0`
- `lower_ticks < upper_ticks`
- `lower_ticks % tick_size == 0` and `upper_ticks % tick_size == 0`

### 2.3 Percent vs whole-number events — no special case in code

The `divider` absorbs the difference. Do **not** add a `×100` anywhere in the backend.

| Contract | Display unit | Tick | `divider` | Display multiplier | `multiplier_micro_per_display_unit` | `tick_value_micro` |
|---|---|---|---|---|---|---|
| CPI YoY | % | 0.01% (1bp) | 100 | $10 per bp = $1,000 per 1% | 1_000_000_000 | 10_000_000 ($10) |
| GDP QoQ | % | 0.01% | 100 | $5 per bp = $500 per 1% | 500_000_000 | 5_000_000 ($5) |
| TTF gas | €/MWh | €0.01 | 100 | $10 per €1 | 10_000_000 | 100_000 ($0.10) |
| Bitcoin | $ | $1 | 1 | $0.01 per $1 | 10_000 | 10_000 ($0.01) |
| Stock | $ | $0.01 | 100 | $1 per $1 | 1_000_000 | 10_000 ($0.01) |
| Electoral votes | votes | 1 | 1 | $10 per vote | 10_000_000 | 10_000_000 ($10) |
| NFP | thousand jobs | 1k | 1 | $2 per 1k | 2_000_000 | 2_000_000 ($2) |

### 2.4 Display conversions (frontend only)

```text
display_price = price_ticks / divider
display_usdc  = amount_micro / 1_000_000
```

---

## 3. Data model

```text
Contract {
  id, name, underlying_source, unit_label,
  divider, tick_size,
  lower_ticks, upper_ticks,
  multiplier_micro_per_display_unit, tick_value_micro,
  state: LISTED | TRADING | HALTED | CLOSED | SETTLED,
  last_trade_time, settlement_value_ticks (nullable), settlement_source_ref
}

Lot {                       -- FIFO lots, one per opening fill
  id, user_id, contract_id,
  side: LONG | SHORT,
  qty_remaining: int,       -- contracts
  entry_ticks: int,         -- fill price
  lock_micro: int,          -- cash paid into escrow for qty_remaining
  opened_at
}

Order {
  id, user_id, contract_id,
  side: BUY | SELL, type: LIMIT | MARKET,
  limit_ticks (effective limit for MARKET, see §5.3),
  qty, qty_filled,
  opening_qty_reserved, closing_qty_reserved,
  hold_micro                -- cash currently held for this order
}

Ledger accounts (per contract where relevant):
  user.available_micro
  user.order_hold_micro
  contract.escrow_micro
```

Position (per user, per contract) is derived: `signed_qty = Σ long lots − Σ short lots`. A user never holds both long and short lots in the same contract at once (buys close shorts first; see §5.4).

---

## 4. Core formulas (all integers, raw ticks)

Let `L = lower_ticks`, `U = upper_ticks`, `tv = tick_value_micro`, `q = qty`, `p = price_ticks`.

### 4.1 Per-fill cash (the only cash rule during trading)

| Action | Cash for the user | Direction |
|---|---|---|
| Buy to **open** long | `(p − L) × q × tv` | user → escrow |
| Sell to **close** long | `(p − L) × q × tv` | escrow → user |
| Sell to **open** short | `(U − p) × q × tv` | user → escrow |
| Buy to **close** short | `(U − p) × q × tv` | escrow → user |

Every fill has two sides; applying this table to both sides automatically keeps the escrow invariant (§8). No average-entry arithmetic, no rounding.

### 4.2 Settlement

```text
final = clamp(settlement_value_ticks, L, U)

long_payout  = (final − L) × q × tv      -- per long lot
short_payout = (U − final) × q × tv      -- per short lot
```

Sum over all lots of both sides equals `escrow_micro` exactly.

### 4.3 Derived P&L (display only)

Using the lot's `entry_ticks`:

```text
long  lot realized on close at p  = (p − entry) × q × tv
short lot realized on close at p  = (entry − p) × q × tv

unrealized (long)  = (mark − entry) × q × tv   summed over lots
unrealized (short) = (entry − mark) × q × tv   summed over lots
mark = clamp(mark_ticks, L, U)

net at settlement (long)  = (final − entry) × q × tv
net at settlement (short) = (entry − final) × q × tv
```

Identity (use as a test): `net = payout − lock`.

### 4.4 Average entry (display only)

```text
cost_basis_ticks = Σ entry_ticks × qty_remaining   (integer)
avg_entry_display = cost_basis_ticks / total_qty / divider   (float OK for display only)
```

Never use `avg_entry` in cash calculations.

---

## 5. Order lifecycle

### 5.1 Validation on placement

Reject if:
- contract state ≠ `TRADING`
- `limit_ticks < L` or `limit_ticks > U`
- `limit_ticks % tick_size != 0`
- `qty <= 0`
- `hold_micro > user.available_micro`

### 5.2 Split the order into closing and opening portions

```text
closable       = current opposite-side position qty − closing_qty already reserved by other open orders
closing_qty    = min(order.qty, max(closable, 0))
opening_qty    = order.qty − closing_qty
```

Reserve `closing_qty` so two orders can't both close the same position.

### 5.3 Hold (only the opening portion needs cash)

```text
BUY  limit: hold = (limit − L) × opening_qty × tv
SELL limit: hold = (U − limit) × opening_qty × tv
```

**Market orders** are converted to IOC limit orders with a protection price:

```text
BUY  market: limit = min(best_ask + protection_ticks, U)
SELL market: limit = max(best_bid − protection_ticks, L)
```

`protection_ticks` is a per-contract config (e.g. 5% of band). If the book is empty, reject the market order. This replaces the current "hold the full band" rule, which over-locks capital and never gets released.

### 5.4 On each fill (qty `f` at price `p`)

1. Consume closing portion first (FIFO against the user's oldest opposite lots), then opening portion.
2. Closing part: credit per §4.1; reduce lots FIFO; decrement `closing_qty_reserved`; compute realized P&L for display.
3. Opening part: move `required = per §4.1` from `order_hold_micro` to `escrow_micro`; create a new lot with `entry_ticks = p`, `lock_micro = required`.
4. **Price-improvement release** (fill better than limit):
   ```text
   BUY : release (limit − p) × f_open × tv   from hold → available
   SELL: release (p − limit) × f_open × tv   from hold → available
   ```
5. A single order can flip a position (e.g. long 3, sell 5 → close 3, open short 2). Steps 1–3 handle this naturally.

### 5.5 Cancel / expiry / IOC remainder

Release `hold_micro` remaining → `available_micro`; release `closing_qty_reserved`.

---

## 6. Settlement lifecycle

1. At `last_trade_time`: state → `CLOSED`; cancel all resting orders (release holds).
2. Admin enters the official value with the source reference (URL/document ID). **Two-person approval** required.
3. Convert to ticks with a documented rounding rule: `round_half_up(value × divider)`. Store raw value and ticks.
4. `final = clamp(ticks, L, U)`.
5. Pay every lot per §4.2 from escrow to user `available_micro`; mark lots closed.
6. Assert `escrow_micro == 0` after payout. If not, halt and alert (should be impossible).
7. State → `SETTLED`.

---

## 7. Fees

Keep fees in a **separate ledger line**, never folded into price or lock:

```text
fee_micro = fee_per_contract_micro × f      (or bps of notional; decide per product)
```

Charged on fill from `available_micro`. Include fees in the net P&L display as a separate line.

---

## 8. Invariants (assert in code + monitor in production)

| # | Invariant | When |
|---|---|---|
| I1 | `escrow_micro == open_interest × (U − L) × tv` | After every fill, settlement step |
| I2 | `Σ long lot qty == Σ short lot qty == open_interest` | After every fill |
| I3 | `Σ lock_micro (all lots) + Σ unrealized(mark) == escrow_micro` | Any time, any mark |
| I4 | Every `lot.entry_ticks ∈ [L, U]` | On lot creation |
| I5 | `user.available_micro ≥ 0`, `order_hold_micro ≥ 0` | After every ledger write |
| I6 | After settlement: `escrow_micro == 0` | Settlement |
| I7 | Σ of all users' net P&L on a settled contract == −Σ fees | Settlement |

Why I3 holds: a long lot's value at mark is `(mark − L)`, a short's is `(U − mark)`; one of each sums to `(U − L)`.

---

## 9. Numeric safety

- All products in `i128` (or BigInt), then check the result fits the storage type before writing.
- Worst-case term: `(U − L) × max_qty × tv`. Validate at listing that `(U − L) × max_position × tv` < `2^63`.
- No floating point anywhere in the backend path. Floats allowed only in UI rendering.

---

## 10. Frontend copy (fixes the current inconsistency)

The old UI showed `(final − entry) × contracts × multiplier` and the backend paid `(final − lower) × …`. **Both were right** — one is net P&L, the other is gross payout. Show both, labelled:

```text
Collateral locked:       (entry − lower) × contracts × multiplier      [long]
                         (upper − entry) × contracts × multiplier      [short]
Max profit:              (upper − entry) × contracts × multiplier      [long]
                         (entry − lower) × contracts × multiplier      [short]
Returned at settlement:  (final − lower) × contracts × multiplier      [long]
                         (upper − final) × contracts × multiplier      [short]
Net P&L:                 (final − entry) × contracts × multiplier      [long]
                         (entry − final) × contracts × multiplier      [short]
```

Here "multiplier" and prices are in **display units** (e.g. 10 USDC per €1, €40.00). Show all money amounts with the `USDC` suffix, not `$`. The backend computes the same numbers in ticks with `tv`.

---

## 11. Changes from current production

| # | Current | Problem | New |
|---|---|---|---|
| C1 | `margin = (entry − lower) × contracts × multiplier_micro_usdc`, with multiplier = 1000 for TTF | Gives $2 instead of $200. Note 1000 micro-USDC is $0.001, not $10 — the stored value may be in **cents** or another unit. | Use `tick_value_micro` (§2). **Audit every catalogue entry**: confirm the unit of the stored multiplier, recompute `tick_value_micro`, check against the contract spec. |
| C2 | Market order holds full band, never released | Locks up to 100% of band unnecessarily | Protection-price IOC + price-improvement release (§5.3, §5.4) |
| C3 | Frontend shows net P&L labelled as payoff | Looks inconsistent with backend | Label both lines (§10) |
| C4 | Live P&L uses `average_entry_ticks` | Fractional average → rounding drift; not needed | FIFO lots + integer cost basis (§4.3, §4.4) |
| C5 | No explicit price-in-band check | Entry below `lower` → negative lock | Validation (§5.1), invariant I4 |
| C6 | Mark price unclamped | Displayed P&L can exceed collateral | `mark = clamp(mark, L, U)` |
| C7 | `tick_size` only validated | — | Keep for validation; scaling comes only from `tv` |

**Migration:** if any live positions were opened with the wrong multiplier, freeze the affected contracts, recompute locks from lots with the corrected `tv`, reconcile user balances against escrow (I1, I3), then reopen. Log every adjustment.

**Open question for the team:** what unit was the catalogue `multiplier` field populated in? (Answer determines the migration conversion.)

---

## 12. Test vectors

### 12.1 TTF — `divider=100`, `tv=100_000`, `L=2000` (€20), `U=8000` (€80)

| Case | Entry | Final | Long lock | Long payout | Long net | Short lock | Short payout | Short net |
|---|---|---|---|---|---|---|---|---|
| Normal | 4000 | 4500 | $200 | $250 | +$50 | $400 | $350 | −$50 |
| Capped | 4000 | 9500 → 8000 | $200 | $600 | +$400 | $400 | $0 | −$400 |
| Floored | 4000 | 1500 → 2000 | $200 | $0 | −$200 | $400 | $600 | +$200 |

Each row: long payout + short payout = $600 = escrow.

### 12.2 CPI — `divider=100`, `tv=10_000_000` ($10/bp), `L=400` (4.00%), `U=600` (6.00%), qty 2

Entry 500 (5.00%), final 550 (5.50%):
- Long lock `(500−400)×2×1e7` = $2,000; short lock $2,000; escrow $4,000
- Long payout `(550−400)×2×1e7` = $3,000 → **net +$1,000**
- Short payout `(600−550)×2×1e7` = $1,000 → **net −$1,000**

### 12.3 Bitcoin — `divider=1`, `tv=10_000` ($0.01 per $1), `L=40000`, `U=80000`, qty 1

Entry 60000, final 63500: long lock $200, payout $235, **net +$35**.

### 12.4 Electoral votes — `divider=1`, `tv=10_000_000` ($10/vote), `L=200`, `U=340`, qty 1

Entry 270, final 286: long lock $700, payout $860, **net +$160**.

### 12.5 Multi-user sequence (tests §4.1 and I1) — TTF, qty 1 each

| Step | Event | Cash flows | Escrow | OI |
|---|---|---|---|---|
| 1 | A buys (open) from B (open short) @ 4000 | A −$200, B −$400 | $600 | 1 |
| 2 | A sells (close) to C (open long) @ 5000 | C −$300, A +$300 (A realized +$100) | $600 | 1 |
| 3 | B buys (close) from C (close) @ 4200 | B +$380 (realized −$20), C +$220 (realized −$80) | $0 | 0 |

Totals: A +$100, B −$20, C −$80 → sum $0 ✓. I1 holds at every step.

Alternative step 3 — settle at 4500 instead: C receives $250 (net −$50), B receives $350 (net −$50); A +$100; sum $0 ✓.

### 12.6 Order hold / release

- BUY limit 4100, qty 1, TTF: hold `(4100−2000)×1e5` = $210. Filled @ 4000 → move $200 to escrow, release $10.
- BUY market, best ask 4000, `protection_ticks=50`: effective limit 4050, hold $205; filled @ 4000 → release $5.
- User long 3, SELL 5 @ 4200: close 3 (credit `(4200−2000)×3×1e5` = $660), open short 2 (hold/lock `(8000−4200)×2×1e5` = $760).

### 12.7 Rejection cases

- Limit price 1900 (< L) → reject
- Limit price 4005 with `tick_size=10` → reject
- Contract with `multiplier_micro_per_display_unit=1_000_050`, `divider=100` → reject at listing (non-integer `tv`)

---

## 13. Out of scope (later phases)

- **Leveraged IM/VM model**: initial margin as a fraction of risk, daily variation margin, maintenance margin calls, release-window IM add-ons, CCP default waterfall. Unbounded payoff becomes possible (band optional).
- **Portfolio/cross-margin** between futures and binaries; market-maker hedging programmes.
- **Overlapping bands / strike ladders** per event.

The per-fill cash rule (§4.1) and ledger design are compatible with the IM/VM model: the only change is that `lock` becomes `initial_margin` and a daily `VM = (F_t − F_{t−1}) × signed_qty × tv` cash flow is added.
