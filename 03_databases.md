# 03 — Database Schemas

One Postgres instance, multiple schemas. **One schema per service. No service reads another service's schema directly.** Cross-service data access is always via gRPC.

All schemas use Postgres 16. Migrations managed via `golang-migrate` per service in `db/migrations/<service>/`.

---

## 1. `refdata` schema

Owned by `refdata-svc`.

```sql
CREATE SCHEMA refdata;
SET search_path TO refdata;

CREATE TYPE contract_kind AS ENUM ('BINARY', 'SCALAR');
CREATE TYPE contract_state AS ENUM (
  'DRAFT', 'LISTED', 'OPEN', 'CLOSED', 'RESOLVING', 'SETTLED', 'CANCELLED'
);

CREATE TABLE series (
  series_ticker      TEXT PRIMARY KEY,
  title              TEXT NOT NULL,
  description        TEXT,
  created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE events (
  event_ticker            TEXT PRIMARY KEY,
  series_ticker           TEXT NOT NULL REFERENCES series(series_ticker),
  title                   TEXT NOT NULL,
  description             TEXT,
  expected_resolution_at  TIMESTAMPTZ NOT NULL,
  created_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE contracts (
  ticker                    TEXT PRIMARY KEY,
  event_ticker              TEXT NOT NULL REFERENCES events(event_ticker),
  series_ticker             TEXT NOT NULL REFERENCES series(series_ticker),
  kind                      contract_kind NOT NULL,
  question                  TEXT,                      -- binaries
  underlying                TEXT,                      -- scalars
  tick_size                 BIGINT NOT NULL CHECK (tick_size > 0),
  min_price_ticks           BIGINT NOT NULL,
  max_price_ticks           BIGINT NOT NULL,
  lower_bound_ticks         BIGINT,                    -- scalars
  upper_bound_ticks         BIGINT,                    -- scalars
  multiplier_micro_usdc     BIGINT,                    -- scalars
  max_order_size            BIGINT NOT NULL,
  position_limit_per_user   BIGINT NOT NULL,
  state                     contract_state NOT NULL DEFAULT 'DRAFT',
  listed_at                 TIMESTAMPTZ,
  open_at                   TIMESTAMPTZ,
  close_at                  TIMESTAMPTZ,
  expected_resolution_at    TIMESTAMPTZ NOT NULL,
  settlement_source         TEXT,
  oracle_policy             TEXT NOT NULL DEFAULT 'ADMIN',
  created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at                TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON contracts(event_ticker);
CREATE INDEX ON contracts(state);

CREATE TABLE contract_state_history (
  id                BIGSERIAL PRIMARY KEY,
  ticker            TEXT NOT NULL REFERENCES contracts(ticker),
  old_state         contract_state,
  new_state         contract_state NOT NULL,
  reason            TEXT,
  changed_by        TEXT,                              -- actor
  changed_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON contract_state_history(ticker, changed_at);
```

---

## 2. `users` schema (shared identity, owned by `auth` subset of `gw-rest` for demo, dedicated `auth-svc` in production)

For the demo, `users` lives alongside `risk` schema since `risk-svc` owns user-level config. In production, this splits out to a dedicated `auth-svc`.

```sql
CREATE SCHEMA users;
SET search_path TO users;

CREATE TABLE users (
  user_id           TEXT PRIMARY KEY,           -- e.g., "u_42"
  email             TEXT UNIQUE NOT NULL,
  display_name      TEXT,
  password_hash     TEXT NOT NULL,              -- argon2id; demo uses bcrypt for simplicity
  kyc_tier          INT NOT NULL DEFAULT 0,
  status            TEXT NOT NULL DEFAULT 'ACTIVE', -- ACTIVE | FROZEN | CLOSED
  is_admin          BOOLEAN NOT NULL DEFAULT false,
  is_mm             BOOLEAN NOT NULL DEFAULT false,
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_keys (
  key_id            TEXT PRIMARY KEY,
  user_id           TEXT NOT NULL REFERENCES users(user_id),
  public_key        BYTEA NOT NULL,             -- ed25519 verify key
  label             TEXT,
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  revoked_at        TIMESTAMPTZ
);
CREATE INDEX ON api_keys(user_id);
```

---

## 3. `ledger` schema

Owned by `ledger-svc`. **Append-only.** No UPDATE/DELETE on entries.

```sql
CREATE SCHEMA ledger;
SET search_path TO ledger;

CREATE TYPE account_type AS ENUM (
  'ASSET', 'LIABILITY', 'EQUITY', 'REVENUE', 'EXPENSE'
);

CREATE TABLE accounts (
  account_id        BIGSERIAL PRIMARY KEY,
  account_code      TEXT NOT NULL UNIQUE,        -- e.g., "LIAB:USER:u_42:CASH"
  account_type      account_type NOT NULL,
  currency          TEXT NOT NULL DEFAULT 'USDC',
  user_id           TEXT,                        -- nullable; null for house accounts
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON accounts(user_id) WHERE user_id IS NOT NULL;

CREATE TABLE transactions (
  tx_id             BIGSERIAL PRIMARY KEY,
  idempotency_key   TEXT NOT NULL UNIQUE,
  reason_code       TEXT NOT NULL,                -- FILL | DEPOSIT | WITHDRAWAL | SETTLEMENT | FEE | HOLD_PLACE | HOLD_RELEASE | HOLD_COMMIT
  metadata          JSONB NOT NULL DEFAULT '{}',
  posted_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON transactions(reason_code);
CREATE INDEX ON transactions(posted_at);

CREATE TABLE entries (
  entry_id              BIGSERIAL PRIMARY KEY,
  tx_id                 BIGINT NOT NULL REFERENCES transactions(tx_id),
  account_id            BIGINT NOT NULL REFERENCES accounts(account_id),
  direction             CHAR(2) NOT NULL CHECK (direction IN ('DR', 'CR')),
  amount_micro_usdc     BIGINT NOT NULL CHECK (amount_micro_usdc > 0),
  running_balance_micro_usdc BIGINT NOT NULL,    -- denormalized per (account, seq)
  account_seq           BIGINT NOT NULL,          -- per-account monotonic
  memo                  TEXT,
  posted_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(account_id, account_seq)
);
CREATE INDEX ON entries(tx_id);
CREATE INDEX ON entries(account_id, entry_id DESC);

-- Enforce balanced transactions via constraint trigger
CREATE OR REPLACE FUNCTION assert_tx_balanced() RETURNS trigger AS $$
DECLARE
  imbalance BIGINT;
BEGIN
  SELECT
    COALESCE(SUM(CASE WHEN direction = 'DR' THEN amount_micro_usdc ELSE 0 END), 0)
  - COALESCE(SUM(CASE WHEN direction = 'CR' THEN amount_micro_usdc ELSE 0 END), 0)
  INTO imbalance
  FROM entries WHERE tx_id = NEW.tx_id;
  IF imbalance <> 0 THEN
    RAISE EXCEPTION 'Ledger transaction % unbalanced by %', NEW.tx_id, imbalance;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER trg_assert_tx_balanced
  AFTER INSERT ON entries
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION assert_tx_balanced();

CREATE TABLE holds (
  hold_id           TEXT PRIMARY KEY,             -- e.g., "hold_<snowflake>"
  user_id           TEXT NOT NULL,
  amount_micro_usdc BIGINT NOT NULL CHECK (amount_micro_usdc >= 0),
  committed_micro_usdc BIGINT NOT NULL DEFAULT 0,
  released_micro_usdc  BIGINT NOT NULL DEFAULT 0,
  reason            TEXT NOT NULL,
  status            TEXT NOT NULL DEFAULT 'ACTIVE', -- ACTIVE | CLOSED
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  closed_at         TIMESTAMPTZ
);
CREATE INDEX ON holds(user_id);
CREATE INDEX ON holds(status);

-- View: current balance per user
CREATE OR REPLACE VIEW user_balances AS
SELECT
  a.user_id,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:CASH'
                    THEN e.running_balance_micro_usdc END), 0) AS cash_micro_usdc,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:HOLDS'
                    THEN e.running_balance_micro_usdc END), 0) AS held_micro_usdc
FROM accounts a
LEFT JOIN LATERAL (
  SELECT running_balance_micro_usdc
  FROM entries
  WHERE account_id = a.account_id
  ORDER BY entry_id DESC LIMIT 1
) e ON true
WHERE a.user_id IS NOT NULL
GROUP BY a.user_id;
```

**House accounts** seeded at boot:
```
ASSET:HOUSE:WALLET                  -- demo: virtual wallet for "fake deposits"
LIAB:UNALLOCATED_DEPOSIT
REVENUE:FEES:TAKER
REVENUE:FEES:MAKER
REVENUE:SETTLEMENT_ROUNDING
LIAB:HOUSE:UNSETTLED_TRADES:<ticker>  -- created on contract creation
```

---

## 4. `orders` schema

Owned by `order-router`.

```sql
CREATE SCHEMA orders;
SET search_path TO orders;

CREATE TYPE order_side AS ENUM ('YES', 'NO', 'LONG', 'SHORT');
CREATE TYPE order_action AS ENUM ('BUY', 'SELL');
CREATE TYPE time_in_force AS ENUM ('GTC', 'IOC', 'FOK');
CREATE TYPE order_status AS ENUM ('OPEN', 'PARTIAL', 'FILLED', 'CANCELLED', 'REJECTED', 'EXPIRED');

CREATE TABLE orders (
  order_id              TEXT PRIMARY KEY,         -- Snowflake
  client_order_id       TEXT,
  user_id               TEXT NOT NULL,
  ticker                TEXT NOT NULL,
  side                  order_side NOT NULL,
  action                order_action NOT NULL,
  price_ticks           BIGINT NOT NULL,
  count                 BIGINT NOT NULL CHECK (count > 0),
  filled_count          BIGINT NOT NULL DEFAULT 0,
  remaining_count       BIGINT NOT NULL,
  tif                   time_in_force NOT NULL,
  post_only             BOOLEAN NOT NULL DEFAULT false,
  reduce_only           BOOLEAN NOT NULL DEFAULT false,
  stp                   TEXT,
  status                order_status NOT NULL,
  hold_id               TEXT,
  avg_fill_price_ticks  BIGINT NOT NULL DEFAULT 0,
  reject_code           TEXT,
  reject_reason         TEXT,
  created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at            TIMESTAMPTZ,
  UNIQUE (user_id, client_order_id)
);
CREATE INDEX ON orders(user_id, status);
CREATE INDEX ON orders(ticker, status);
CREATE INDEX ON orders(status) WHERE status IN ('OPEN', 'PARTIAL'); -- for ME warm restart

CREATE TABLE fills (
  fill_id           TEXT PRIMARY KEY,
  maker_order_id    TEXT NOT NULL REFERENCES orders(order_id),
  taker_order_id    TEXT NOT NULL REFERENCES orders(order_id),
  maker_user_id     TEXT NOT NULL,
  taker_user_id     TEXT NOT NULL,
  ticker            TEXT NOT NULL,
  price_ticks       BIGINT NOT NULL,
  count             BIGINT NOT NULL CHECK (count > 0),
  aggressor_side    order_action NOT NULL,
  maker_fee_micro_usdc BIGINT NOT NULL DEFAULT 0,
  taker_fee_micro_usdc BIGINT NOT NULL DEFAULT 0,
  ticker_seq        BIGINT NOT NULL,             -- monotonic per ticker
  global_seq        BIGINT NOT NULL,             -- monotonic across me-core
  ts                TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(ticker, ticker_seq)
);
CREATE INDEX ON fills(ticker, ts DESC);
CREATE INDEX ON fills(maker_user_id, ts DESC);
CREATE INDEX ON fills(taker_user_id, ts DESC);
```

---

## 5. `risk` schema

Owned by `risk-svc`. Uses Redis for hot path; Postgres is durable source of truth.

```sql
CREATE SCHEMA risk;
SET search_path TO risk;

CREATE TABLE user_limits (
  user_id                     TEXT PRIMARY KEY,
  kyc_tier                    INT NOT NULL DEFAULT 0,
  max_order_size_micro_usdc   BIGINT NOT NULL DEFAULT 100000000,    -- $100
  daily_loss_limit_micro_usdc BIGINT NOT NULL DEFAULT 1000000000,   -- $1000
  orders_per_second_limit     INT NOT NULL DEFAULT 5,
  updated_at                  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE contract_position_limits (
  user_id           TEXT NOT NULL,
  ticker            TEXT NOT NULL,
  max_qty           BIGINT NOT NULL,
  PRIMARY KEY (user_id, ticker)
);

CREATE TABLE working_orders_summary (
  user_id           TEXT NOT NULL,
  ticker            TEXT NOT NULL,
  side              TEXT NOT NULL,                 -- YES/NO/LONG/SHORT
  total_qty         BIGINT NOT NULL DEFAULT 0,
  total_max_loss_micro_usdc BIGINT NOT NULL DEFAULT 0,
  updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, ticker, side)
);
```

---

## 6. `position` schema

Owned by `position-svc`.

```sql
CREATE SCHEMA position;
SET search_path TO position;

CREATE TABLE positions (
  user_id                     TEXT NOT NULL,
  ticker                      TEXT NOT NULL,
  net_qty                     BIGINT NOT NULL DEFAULT 0,    -- signed
  avg_cost_micro_usdc         BIGINT NOT NULL DEFAULT 0,
  realized_pnl_micro_usdc     BIGINT NOT NULL DEFAULT 0,
  last_trade_seq              BIGINT NOT NULL DEFAULT 0,    -- per-ticker
  updated_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, ticker)
);
CREATE INDEX ON positions(ticker) WHERE net_qty != 0;

CREATE TABLE position_history (
  id                BIGSERIAL PRIMARY KEY,
  user_id           TEXT NOT NULL,
  ticker            TEXT NOT NULL,
  net_qty_before    BIGINT NOT NULL,
  net_qty_after     BIGINT NOT NULL,
  fill_id           TEXT NOT NULL,
  ts                TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE open_interest (
  ticker            TEXT PRIMARY KEY,
  total_open_long   BIGINT NOT NULL DEFAULT 0,
  total_open_short  BIGINT NOT NULL DEFAULT 0,
  updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

---

## 7. `oracle` schema

Owned by `oracle-svc`.

```sql
CREATE SCHEMA oracle;
SET search_path TO oracle;

CREATE TYPE resolution_status AS ENUM (
  'PENDING', 'PROPOSED', 'FINALIZED', 'DISPUTED'
);

CREATE TABLE attestations (
  id                BIGSERIAL PRIMARY KEY,
  event_ticker      TEXT NOT NULL,
  attestor_id       TEXT NOT NULL,
  source            TEXT NOT NULL,
  numeric_value     BIGINT,
  categorical_value TEXT,
  signature         BYTEA,
  observed_at       TIMESTAMPTZ NOT NULL,
  received_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(event_ticker, attestor_id, source)
);
CREATE INDEX ON attestations(event_ticker);

CREATE TABLE resolutions (
  event_ticker          TEXT PRIMARY KEY,
  status                resolution_status NOT NULL DEFAULT 'PENDING',
  numeric_value         BIGINT,
  categorical_value     TEXT,
  proposed_at           TIMESTAMPTZ,
  finalized_at          TIMESTAMPTZ,
  challenge_window_ends_at TIMESTAMPTZ,
  attestor_count        INT NOT NULL DEFAULT 0,
  required_quorum       INT NOT NULL DEFAULT 1,
  created_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oracle_keys (
  attestor_id       TEXT PRIMARY KEY,
  public_key        BYTEA NOT NULL,
  active            BOOLEAN NOT NULL DEFAULT true,
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

---

## 8. `settlement` schema

Owned by `settlement-svc`.

```sql
CREATE SCHEMA settlement;
SET search_path TO settlement;

CREATE TABLE settlements (
  ticker                              TEXT PRIMARY KEY,
  event_ticker                        TEXT NOT NULL,
  numeric_value                       BIGINT,
  categorical_value                   TEXT,
  winner_payout_per_contract_micro_usdc BIGINT NOT NULL,
  total_payout_micro_usdc             BIGINT NOT NULL DEFAULT 0,
  positions_settled                   INT NOT NULL DEFAULT 0,
  status                              TEXT NOT NULL DEFAULT 'PENDING', -- PENDING | IN_PROGRESS | COMPLETE | FAILED
  started_at                          TIMESTAMPTZ,
  completed_at                        TIMESTAMPTZ
);

CREATE TABLE settlement_payouts (
  id                BIGSERIAL PRIMARY KEY,
  ticker            TEXT NOT NULL REFERENCES settlements(ticker),
  user_id           TEXT NOT NULL,
  position_qty      BIGINT NOT NULL,
  payout_micro_usdc BIGINT NOT NULL,
  ledger_tx_id      BIGINT,                       -- FK conceptually; ledger owns
  idempotency_key   TEXT UNIQUE NOT NULL,         -- "settlement:<ticker>:<user>"
  posted_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON settlement_payouts(ticker);
CREATE INDEX ON settlement_payouts(user_id);
```

---

## 9. `audit` schema

Owned by `audit-svc`.

```sql
CREATE SCHEMA audit;
SET search_path TO audit;

CREATE TABLE events (
  event_id          BIGSERIAL PRIMARY KEY,
  event_seq         BIGINT UNIQUE NOT NULL,        -- monotonic across all services
  service           TEXT NOT NULL,
  type              TEXT NOT NULL,
  actor             TEXT NOT NULL,                 -- user_id, "service:<name>", "admin:<email>"
  subject           TEXT,                          -- ticker, order_id, user_id, etc.
  payload           JSONB NOT NULL,
  prev_hash         BYTEA,                         -- production: hash chain
  hash              BYTEA,
  trace_id          TEXT,
  ts                TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON events(service, ts DESC);
CREATE INDEX ON events(actor, ts DESC);
CREATE INDEX ON events(subject, ts DESC);
CREATE INDEX ON events(type, ts DESC);

-- Sequence allocator
CREATE SEQUENCE event_seq_gen;
```

---

## 10. Seed data

### `db/seed/contracts.sql`

```sql
-- Series
INSERT INTO refdata.series (series_ticker, title, description) VALUES
  ('RBI-RATEDECISION', 'RBI Repo Rate Decisions',
   'Binary contracts on RBI Monetary Policy Committee rate decisions'),
  ('INDIA-CPI', 'India CPI YoY Inflation',
   'Scalar futures on India MoSPI Consumer Price Inflation YoY readings');

-- Events
INSERT INTO refdata.events (event_ticker, series_ticker, title, description, expected_resolution_at) VALUES
  ('RBI-JUN26', 'RBI-RATEDECISION', 'RBI June 2026 MPC Meeting',
   'Outcome of the June 2026 RBI Monetary Policy Committee meeting',
   '2026-06-06 11:00:00+00'),
  ('INDIA-CPI-JUN26', 'INDIA-CPI', 'India CPI June 2026 Print',
   'India CPI-Combined YoY % for June 2026, as released by MoSPI',
   '2026-07-12 12:30:00+00');

-- Contracts (binary)
INSERT INTO refdata.contracts (
  ticker, event_ticker, series_ticker, kind, question,
  tick_size, min_price_ticks, max_price_ticks,
  max_order_size, position_limit_per_user, state,
  open_at, close_at, expected_resolution_at,
  settlement_source, oracle_policy
) VALUES (
  'RBI-JUN26-CUT25', 'RBI-JUN26', 'RBI-RATEDECISION', 'BINARY',
  'Will the RBI cut repo rate by 25 bps or more at the June 2026 MPC?',
  1, 1, 99,
  100000, 250000, 'OPEN',
  '2026-04-01 00:00:00+00', '2026-06-06 08:30:00+00', '2026-06-06 11:00:00+00',
  'https://rbi.org.in/Scripts/BS_PressReleaseDisplay.aspx', 'MULTI_SOURCE_ATTEST'
);

-- Contracts (scalar)
-- price_ticks = bps (200..800), multiplier = $1 per 0.01% range fill
INSERT INTO refdata.contracts (
  ticker, event_ticker, series_ticker, kind, underlying,
  tick_size, min_price_ticks, max_price_ticks,
  lower_bound_ticks, upper_bound_ticks, multiplier_micro_usdc,
  max_order_size, position_limit_per_user, state,
  open_at, close_at, expected_resolution_at,
  settlement_source, oracle_policy
) VALUES (
  'INDIA-CPI-JUN26-SCALAR', 'INDIA-CPI-JUN26', 'INDIA-CPI', 'SCALAR',
  'India CPI-C YoY % (basis points)',
  1, 200, 800,
  200, 800, 1000000,
  100000, 250000, 'OPEN',
  '2026-04-01 00:00:00+00', '2026-07-12 12:00:00+00', '2026-07-12 12:30:00+00',
  'https://mospi.gov.in/cpi', 'SINGLE_SOURCE'
);
```

### `db/seed/demo_users.sql`

```sql
-- Passwords hashed with bcrypt for demo (all "demo1234")
INSERT INTO users.users (user_id, email, display_name, password_hash, kyc_tier, is_admin, is_mm) VALUES
  ('u_retail_1', 'retail@demo.sarvaex.com', 'Demo Retail',
   '$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy', 1, false, false),
  ('u_inst_1', 'inst@demo.sarvaex.com', 'Demo Institutional',
   '$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy', 2, false, false),
  ('u_mm_1', 'mm@demo.sarvaex.com', 'Demo MM Bot',
   '$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy', 2, false, true),
  ('u_admin', 'admin@demo.sarvaex.com', 'Demo Admin',
   '$2a$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy', 99, true, false);

-- Seed risk limits
INSERT INTO risk.user_limits (user_id, kyc_tier, max_order_size_micro_usdc, daily_loss_limit_micro_usdc, orders_per_second_limit) VALUES
  ('u_retail_1', 1, 100000000, 1000000000, 5),
  ('u_inst_1', 2, 10000000000, 100000000000, 50),
  ('u_mm_1', 2, 100000000000, 1000000000000, 200);

-- Seed cash balances (admin-credited fake deposits)
-- This will be done via the ledger admin endpoint at boot time, see seed-demo-data.sh
```

---

## 11. Schema ownership map (enforced via Postgres roles)

| Service | Owns (R/W) | Reads-via-gRPC | Reads-via-NATS-event |
|---|---|---|---|
| `gw-rest` | — | refdata, orders, position, ledger, risk | — |
| `gw-ws` | — | refdata | md.book, md.trade, md.ticker, ledger.events, exec.* |
| `order-router` | orders | refdata, risk, ledger, me-core | exec.* |
| `risk-svc` | risk | refdata | exec.*, ledger.events |
| `ledger-svc` | ledger, users | — | — |
| `me-core` | (in-memory) | refdata (boot) | — |
| `position-svc` | position | — | md.trade.* |
| `refdata-svc` | refdata | — | — |
| `oracle-svc` | oracle | refdata | — |
| `settlement-svc` | settlement | refdata, oracle, position, ledger | resolution.finalized |
| `audit-svc` | audit | — | audit.events |
| `admin-svc` | — | all services via admin RPCs | — |

In production, each service connects with a dedicated Postgres role that has `USAGE` only on its own schema and no access to others. In demo, all services use the same role to keep things simple — but the **discipline is enforced in code review**.
