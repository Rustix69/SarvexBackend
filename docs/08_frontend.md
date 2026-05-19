# 08 — Frontend (`web/`)

The investor demo is a frontend demo. Architecture and code quality matter as much as the backend.

---

## 1. Stack

- **Framework:** Next.js 14, App Router
- **Language:** TypeScript (strict mode)
- **Styling:** Tailwind CSS + shadcn/ui (Radix primitives)
- **State:** Zustand for client state, TanStack Query for server state
- **Charts:** Recharts (price/depth charts), lightweight-charts (TradingView-style if needed for polish)
- **Real-time:** native WebSocket
- **Forms:** react-hook-form + zod
- **Date/time:** date-fns
- **Money formatting:** custom utils that work in micro_usdc int64 throughout, formatting only at display layer

---

## 2. App layout

```
web/
├── app/
│   ├── (marketing)/               # Public landing
│   │   ├── page.tsx
│   │   └── layout.tsx
│   ├── (app)/                     # Authenticated app
│   │   ├── layout.tsx             # Header, sidebar, auth gate
│   │   ├── markets/
│   │   │   ├── page.tsx           # Markets list
│   │   │   └── [ticker]/
│   │   │       └── page.tsx       # Trading screen
│   │   ├── portfolio/
│   │   │   ├── page.tsx           # Positions
│   │   │   └── orders/page.tsx    # Blotter
│   │   ├── account/
│   │   │   ├── page.tsx           # Balance, history
│   │   │   └── deposits/page.tsx  # Deposit screen
│   │   └── admin/                 # Admin-only routes
│   │       ├── layout.tsx         # Role gate
│   │       ├── page.tsx
│   │       ├── contracts/page.tsx
│   │       ├── users/page.tsx
│   │       ├── deposits/page.tsx
│   │       ├── oracle/page.tsx
│   │       └── audit/page.tsx
│   ├── login/page.tsx
│   ├── api/                       # Demo: a few BFF routes for SSR; mostly empty
│   ├── layout.tsx                 # Root layout
│   └── globals.css
│
├── components/
│   ├── ui/                        # shadcn primitives
│   ├── trading/
│   │   ├── OrderBook.tsx
│   │   ├── DepthChart.tsx
│   │   ├── PriceChart.tsx
│   │   ├── OrderTicket.tsx
│   │   ├── ContractHeader.tsx
│   │   ├── TradeTape.tsx
│   │   └── PositionPanel.tsx
│   ├── portfolio/
│   │   ├── OpenOrdersTable.tsx
│   │   ├── PositionsTable.tsx
│   │   ├── FillsHistory.tsx
│   │   └── PnLCard.tsx
│   ├── account/
│   │   ├── BalanceCard.tsx
│   │   ├── DepositForm.tsx
│   │   └── HistoryTable.tsx
│   └── admin/
│       ├── ContractsAdmin.tsx
│       ├── UsersAdmin.tsx
│       ├── OracleAdmin.tsx
│       ├── DepositCredit.tsx
│       └── AuditLog.tsx
│
├── lib/
│   ├── api.ts                     # REST client
│   ├── ws.ts                      # WebSocket client w/ reconnect & resync
│   ├── auth.ts                    # Login, JWT mgmt, refresh
│   ├── stores/
│   │   ├── auth.ts                # current user
│   │   ├── orderbook.ts           # one store per ticker (Map)
│   │   ├── orders.ts              # user's open orders
│   │   ├── positions.ts
│   │   └── balance.ts
│   ├── format/
│   │   ├── money.ts               # micro_usdc → display
│   │   ├── price.ts               # ticks → display
│   │   └── time.ts
│   └── types.ts                   # TS types mirroring proto
│
├── public/
│   ├── logo.svg
│   ├── favicon.ico
│   └── og.png
│
├── styles/
│   └── tokens.css                 # Design tokens
│
├── package.json
├── tsconfig.json
├── tailwind.config.ts
└── Dockerfile
```

---

## 3. Design language

This is critical for the investor demo. Looking generic will lose 30% of the impact.

### 3.1 Tokens

```css
/* styles/tokens.css */
:root {
  /* Brand */
  --color-bg-primary: #0a0e14;
  --color-bg-surface: #11161f;
  --color-bg-surface-elevated: #161c28;
  --color-bg-surface-hover: #1c2433;
  --color-border-subtle: #1f2937;
  --color-border-default: #2d3748;

  /* Semantic */
  --color-yes: #00d4a8;       /* teal — the "long" / "yes" color */
  --color-no:  #ff6b6b;       /* coral — the "short" / "no" color */
  --color-neutral: #a0aec0;
  --color-warn: #fbbf24;

  /* Text */
  --color-text-primary: #e8ecf2;
  --color-text-secondary: #94a3b8;
  --color-text-muted: #64748b;

  /* Accent */
  --color-accent: #7c5cff;     /* Sarvex purple */
  --color-accent-glow: rgba(124, 92, 255, 0.4);

  /* Type */
  --font-display: "Space Grotesk", system-ui, sans-serif;
  --font-mono: "JetBrains Mono", "SF Mono", monospace;
  --font-body: "Inter", system-ui, sans-serif;

  /* Spacing/sizing on 4px grid */
  /* radii: 4 / 8 / 12 / 16 */
  /* Animation: 150ms ease, 250ms ease-out */
}
```

### 3.2 Anti-patterns to avoid

- No purple-blue gradients on landing page (too generic SaaS).
- No "abstract 3D mesh" hero imagery.
- No emoji icons. Lucide icons only.
- No "trusted by" carousel with logos we don't have.
- No shadcn defaults left untouched. Customize them or it looks AI-generated.

### 3.3 What to lean into

- Monospace numbers everywhere prices, sizes, and balances appear.
- Tight grids; dense data layouts (this is a trading interface, not a marketing page).
- Subtle motion on order book updates (flash on price change, fade after 300ms).
- A "latency" indicator in the header showing live order-ack p99 in milliseconds — this is a credibility flex for technical investors.
- Real-time count of "Active markets / Volume today / Open interest" on the markets page.

---

## 4. Real-time data flow

```
WebSocket connection (single, persistent)
  │
  ├─ md.book.<ticker>         → useOrderBookStore(ticker).applyDelta()
  ├─ md.trade.<ticker>        → useOrderBookStore(ticker).addTrade()
  ├─ md.ticker.<ticker>       → useOrderBookStore(ticker).setTicker()
  ├─ md.lifecycle.<ticker>    → useMarketsStore.transitionContract()
  ├─ exec.user.<user_id>      → useOrdersStore.applyEvent()
  ├─ exec.fills.user.<u>      → useOrdersStore.addFill()
  └─ ledger.balance.user.<u>  → useBalanceStore.set()

Components subscribe to these stores via Zustand selectors.
```

### 4.1 The WS client

```typescript
// lib/ws.ts
type Sid = number;
type Subscription = {
  channel: string;
  scope: Record<string, string>;
  onMessage: (msg: any, seq: number) => void;
  onResync: () => void;       // gap detected
  lastSeq: number;
};

class WSClient {
  private ws: WebSocket | null = null;
  private subs = new Map<Sid, Subscription>();
  private nextSid = 1;
  private reconnectAttempts = 0;
  private heartbeat: number | null = null;

  connect(token: string | null) {
    const url = token
      ? `${process.env.NEXT_PUBLIC_WS_URL}?token=${token}`
      : process.env.NEXT_PUBLIC_WS_URL!;
    this.ws = new WebSocket(url);
    this.ws.onopen = this.onOpen;
    this.ws.onmessage = this.onMessage;
    this.ws.onclose = this.onClose;
    this.ws.onerror = (e) => console.error("ws error", e);
  }

  subscribe(channel: string, scope: Record<string, string>,
            onMessage: (msg: any, seq: number) => void): Sid {
    const sid = this.nextSid++;
    this.subs.set(sid, { channel, scope, onMessage, onResync: () => {}, lastSeq: 0 });
    this.send({ id: sid, cmd: "subscribe", params: { channel, ...scope } });
    return sid;
  }

  unsubscribe(sid: Sid) {
    this.send({ id: sid, cmd: "unsubscribe" });
    this.subs.delete(sid);
  }

  private onMessage = (e: MessageEvent) => {
    const evt = JSON.parse(e.data);
    if (evt.type === "subscribed" || evt.type === "welcome") return;
    const sub = this.subs.get(evt.sid);
    if (!sub) return;
    if (evt.seq && sub.lastSeq && evt.seq !== sub.lastSeq + 1) {
      // Gap; request resync
      this.send({ id: evt.sid, cmd: "resync", params: { sid: evt.sid } });
      sub.lastSeq = 0;
      sub.onResync();
      return;
    }
    sub.lastSeq = evt.seq ?? sub.lastSeq;
    sub.onMessage(evt.msg, evt.seq);
  };

  private onClose = () => {
    if (this.heartbeat) clearInterval(this.heartbeat);
    setTimeout(() => this.connect(getToken()), Math.min(1000 * 2 ** this.reconnectAttempts++, 30000));
  };

  private onOpen = () => {
    this.reconnectAttempts = 0;
    // Re-subscribe everything
    for (const [sid, sub] of this.subs) {
      this.send({ id: sid, cmd: "subscribe", params: { channel: sub.channel, ...sub.scope } });
      sub.lastSeq = 0;
      sub.onResync();
    }
    this.heartbeat = window.setInterval(() => this.send({ cmd: "ping" }), 15000) as any;
  };

  private send(o: any) {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(o));
  }
}

export const wsClient = new WSClient();
```

### 4.2 Order book store

```typescript
// lib/stores/orderbook.ts
import { create } from "zustand";

type Level = { price: number; qty: number };
type State = {
  bids: Level[];                     // descending price
  asks: Level[];                     // ascending price
  lastTradePrice: number | null;
  ticker: { best_bid: Level; best_ask: Level } | null;
  trades: Array<{ price: number; qty: number; side: "BUY"|"SELL"; ts: number }>;
  ready: boolean;
};

type Actions = {
  applySnapshot(s: { bids: Level[]; asks: Level[]; seq: number }): void;
  applyDelta(d: { deltas: Array<{ side: "YES"|"NO"|"LONG"|"SHORT"; price_ticks: number; new_total_qty: number }> }): void;
  addTrade(t: any): void;
  reset(): void;
};

const orderBookStores = new Map<string, ReturnType<typeof create<State & Actions>>>();

export function useOrderBookStore(ticker: string) {
  if (!orderBookStores.has(ticker)) {
    const store = create<State & Actions>((set, get) => ({
      bids: [], asks: [], lastTradePrice: null, ticker: null, trades: [], ready: false,
      applySnapshot: (s) => set({
        bids: s.bids.sort((a,b) => b.price - a.price),
        asks: s.asks.sort((a,b) => a.price - b.price),
        ready: true,
      }),
      applyDelta: (d) => set(state => {
        const bids = [...state.bids], asks = [...state.asks];
        for (const delta of d.deltas) {
          const isBid = delta.side === "YES" || delta.side === "LONG";
          const list = isBid ? bids : asks;
          const idx = list.findIndex(l => l.price === delta.price_ticks);
          if (delta.new_total_qty === 0) {
            if (idx >= 0) list.splice(idx, 1);
          } else if (idx >= 0) {
            list[idx] = { price: delta.price_ticks, qty: delta.new_total_qty };
          } else {
            list.push({ price: delta.price_ticks, qty: delta.new_total_qty });
            if (isBid) bids.sort((a,b) => b.price - a.price);
            else asks.sort((a,b) => a.price - b.price);
          }
        }
        return { bids, asks };
      }),
      addTrade: (t) => set(state => ({
        trades: [{ price: t.price_ticks, qty: t.count, side: t.aggressor_side, ts: t.ts }, ...state.trades].slice(0, 100),
        lastTradePrice: t.price_ticks,
      })),
      reset: () => set({ bids: [], asks: [], trades: [], lastTradePrice: null, ready: false }),
    }));
    orderBookStores.set(ticker, store);
  }
  return orderBookStores.get(ticker)!;
}
```

### 4.3 The trading screen component composition

```typescript
// app/(app)/markets/[ticker]/page.tsx
"use client";

export default function TradingScreen({ params }: { params: { ticker: string } }) {
  const ticker = params.ticker;

  useEffect(() => {
    const store = useOrderBookStore(ticker);
    const sid = wsClient.subscribe("orderbook_snapshot", { ticker }, (msg, seq) => store.getState().applySnapshot({ ...msg, seq }));
    const sid2 = wsClient.subscribe("orderbook_delta",  { ticker }, (msg) => store.getState().applyDelta(msg));
    const sid3 = wsClient.subscribe("trades",            { ticker }, (msg) => store.getState().addTrade(msg));
    return () => { wsClient.unsubscribe(sid); wsClient.unsubscribe(sid2); wsClient.unsubscribe(sid3); };
  }, [ticker]);

  return (
    <div className="grid grid-cols-12 gap-3 h-[calc(100vh-64px)] p-3">
      <div className="col-span-8 grid grid-rows-[auto_1fr_280px] gap-3">
        <ContractHeader ticker={ticker} />
        <PriceChart ticker={ticker} />
        <TradeTape ticker={ticker} />
      </div>
      <div className="col-span-4 grid grid-rows-[1fr_auto_auto] gap-3">
        <OrderBook ticker={ticker} />
        <OrderTicket ticker={ticker} />
        <PositionPanel ticker={ticker} />
      </div>
    </div>
  );
}
```

---

## 5. Screens

### 5.1 Markets list (`/markets`)

- Grid of contract cards. Each card shows: question (binary) or underlying (scalar), current best bid/ask, 24h volume, time to close.
- Filter chips: ALL / OPEN / CLOSED.
- Search by ticker or question text.

### 5.2 Trading screen (`/markets/[ticker]`)

Layout for the **investor demo** (this is what they'll spend 60% of their time looking at):
- Left 2/3: chart on top, trade tape below.
- Right 1/3: order book, then order ticket, then "Your position in this contract" panel.
- Header: ticker, question, expected resolution time, state badge, oracle policy badge, last trade price, 24h change.
- The order book renders with bids descending on top, asks ascending on bottom, mid-spread row in between.
- Hovering a price level highlights cumulative size.

### 5.3 Portfolio (`/portfolio`)

- Stats row: total balance, total unrealized PnL, total realized PnL today, total in holds.
- Open positions table (sortable, filterable).
- Open orders blotter (live updates via WS).

### 5.4 Account (`/account`)

- Balance breakdown card: available, in holds, total.
- History table: every ledger entry (paginated, filter by reason_code).
- "Deposit USDC" button (demo: opens a modal with "Awaiting deposit" → fake credit happens via admin or scripted; production: shows actual deposit address).

### 5.5 Admin (`/admin/*`)

Internal-only. **Critical for the demo's settlement scene.**

- **Contracts:** list, transition state, create new from template.
- **Users:** list, freeze/unfreeze, view balance + positions, credit deposit (demo button).
- **Oracle:** for each contract approaching close: "Resolve" button → modal with outcome picker. Tied to `Oracle.AdminForceResolution`.
- **Audit:** live tail of audit events, filter by service/type/actor.
- **Operations:** "Reset demo state" button (only in `DEMO_MODE=true` env).

---

## 6. The order ticket UX

This is the most important screen interaction.

```
┌─────────────────────────────────────┐
│  Order Ticket                       │
├─────────────────────────────────────┤
│   [YES]   [ NO ]    ← side toggle   │
│                                     │
│  Price        Count                 │
│  [____¢]      [_______]             │
│                                     │
│  TIF:  [GTC] [IOC] [FOK]            │
│  ☐ Post-only                        │
│                                     │
│  ───────────────────────            │
│  Max loss:        $62.00            │
│  Max payout:      $100.00           │
│  Required hold:   $62.00            │
│  Available:       $1,000.00         │
│                                     │
│  [    Place Buy YES order    ]      │
└─────────────────────────────────────┘
```

- Side toggle is two big buttons; YES is teal, NO is coral. Selecting one sets price field default to current best ask (for buy) or bid (for sell).
- Numbers in the summary update live as user types.
- Submit button is full-width, colored to match the selected side.
- On submit: optimistic update (order appears in blotter immediately with "Pending"), then resolves to "Open" or shows error toast.

Keyboard shortcuts:
- `Y` / `N` to switch side
- `Tab` to navigate price → count
- `Enter` to submit
- `Esc` to cancel/clear

---

## 7. State of contract lifecycle UX

When a contract transitions state (LISTED → OPEN → CLOSED → RESOLVING → SETTLED), the trading screen reacts:
- **LISTED:** banner "Opens at HH:MM"; order ticket disabled.
- **OPEN:** normal trading.
- **CLOSED:** banner "Trading closed. Resolution expected at HH:MM"; order ticket disabled but cancel still works for any leftover (there shouldn't be any if close handled cleanly).
- **RESOLVING:** banner "Oracle in progress"; show attestation count if `MULTI_SOURCE_ATTEST` (production).
- **SETTLED:** banner "Settled at HH:MM:SS with outcome X. Payouts complete."; position panel shows "Settled +$N USDC".

These transitions are visual events worth showing in the investor demo. Add a subtle flash animation when state changes (1s pulse on the state badge).

---

## 8. Money/price formatting

```typescript
// lib/format/money.ts
export function formatMicroUSDC(amount: bigint | number): string {
  const value = typeof amount === "bigint" ? amount : BigInt(amount);
  const negative = value < 0n;
  const abs = negative ? -value : value;
  const dollars = abs / 1_000_000n;
  const cents = (abs % 1_000_000n) / 10_000n;
  const formatted = `${dollars.toLocaleString()}.${cents.toString().padStart(2, "0")}`;
  return `${negative ? "-" : ""}$${formatted}`;
}

// lib/format/price.ts
export function formatBinaryPriceTicks(ticks: number): string {
  // Binary: ticks = cents 1..99 → "0.62¢" displayed as "62¢" or "$0.62"
  return `${ticks}¢`;
}
export function formatScalarPriceTicks(ticks: number, lower: number, upper: number, unit: string): string {
  // Scalar: ticks are in unit (e.g., bps for CPI); display in native units
  return `${(ticks / 100).toFixed(2)}${unit}`;
}
```

All money flows through Number → bigint at the API boundary, stays bigint until display. No client-side arithmetic on dollars and cents.

---

## 9. Auth flow

1. User opens `/login`, enters email + password.
2. `POST /v1/auth/login` returns `{token, user}`.
3. Token stored in: `localStorage` (demo) / `httpOnly` cookie (production).
4. WSClient reconnects with new token.
5. Protected pages check token validity in layout; redirect to `/login` if missing/expired.

Demo seeds 4 accounts (retail, institutional, mm, admin) all with password `demo1234`. The login page shows these credentials as a hint for investors / dev users.

---

## 10. Performance considerations

- **No suspense waterfalls.** Trading screen kicks off all data fetches in parallel in `useEffect`.
- **Order book renders virtualized** if > 50 levels per side (rare in demo but defensive).
- **Memoize price level rows** so a delta to one level doesn't re-render all rows.
- **Debounce ticker updates** at 50ms (display) even though server emits at 100ms+.
- **Avoid date-fns on every render**; precompute and cache.

---

## 11. Polish checklist for the investor demo

- [ ] Logo + favicon (replace defaults)
- [ ] Marketing landing page that's short, confident, and not generic
- [ ] Color-blind safe palette (YES/NO are teal+coral, not green+red)
- [ ] Empty states with helpful copy, not blank divs
- [ ] Loading skeletons that match real content shape
- [ ] Toast notifications for order accepted/rejected (not blocking modals)
- [ ] Error boundaries on every screen
- [ ] "About this demo" link in footer linking to a 1-pager that frames what's real vs stubbed
- [ ] Live "Latency" indicator in header showing real ms (queries `/v1/health/metrics`)
- [ ] Settled contracts show a small confetti animation (subtle, 500ms) — only on settlement screen, not in production

---

## 12. What's deliberately not built

- Mobile responsive (it's a trading app for desktop; this is fine through Phase 3)
- Charting beyond what Recharts offers (no TradingView integration)
- Push notifications
- Email notifications
- Multi-language support (English only)
- Dark/light theme toggle (dark only)
- Account management beyond view (no password change, no email change)
- Multi-factor auth UI (production phase)
