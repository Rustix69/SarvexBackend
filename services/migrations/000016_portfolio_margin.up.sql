-- Stage 1 cross margin (model C3): fully funded portfolio margin per event.
--
-- Additive only. Every existing event stays ISOLATED (today's per-order holds
-- and per-ticker escrow). Only events listed as PORTFOLIO use the account-level
-- margin tables below. See the design doc "Sarvex Cross Margin (Stage 1, C3)".

-- ---------------------------------------------------------------- refdata

ALTER TABLE refdata.events
  ADD COLUMN IF NOT EXISTS margin_model TEXT NOT NULL DEFAULT 'ISOLATED',
  ADD COLUMN IF NOT EXISTS outcome_kind TEXT NOT NULL DEFAULT 'NUMERIC',
  -- CM-04: kill switch, separate from margin_model. When true, new orders are
  -- rejected (EVENT_HALTED); cancels, matched fills and settlement continue.
  ADD COLUMN IF NOT EXISTS trading_halted BOOLEAN NOT NULL DEFAULT FALSE,
  -- CM-06: raw outcome ticks per display unit. Every future's divider and every
  -- binary payoff_rule must use it before a PORTFOLIO event may open.
  ADD COLUMN IF NOT EXISTS outcome_divider BIGINT,
  -- CM-11: contractual outcome grid { outcome_anchor + n * outcome_step }, within
  -- [outcome_min, outcome_max] when set. The settlement rule must state the
  -- rounding to outcome_step. min/max only for naturally bounded or clamped outcomes.
  ADD COLUMN IF NOT EXISTS outcome_step BIGINT NOT NULL DEFAULT 1,
  ADD COLUMN IF NOT EXISTS outcome_anchor BIGINT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS outcome_min BIGINT,
  ADD COLUMN IF NOT EXISTS outcome_max BIGINT;

ALTER TABLE refdata.contracts
  -- Binary YES condition on the event's outcome axis:
  --   {"type":"range","lo":341,"hi":null}          YES iff lo <= x <= hi (null = open end)
  --   {"type":"categories","yes_values":["HIKE_25"]} YES iff the category is listed
  -- NULL for futures (the band defines their payoff) and for binaries not yet audited.
  ADD COLUMN IF NOT EXISTS payoff_rule JSONB,
  -- Settlement value of a winning binary in price ticks (100 ticks = 1 USDC).
  ADD COLUMN IF NOT EXISTS payout_ticks BIGINT NOT NULL DEFAULT 100;

DO $$ BEGIN
  ALTER TABLE refdata.events ADD CONSTRAINT events_margin_model_check
    CHECK (margin_model IN ('ISOLATED', 'PORTFOLIO'));
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE refdata.events ADD CONSTRAINT events_outcome_kind_check
    CHECK (outcome_kind IN ('NUMERIC', 'CATEGORICAL'));
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE refdata.events ADD CONSTRAINT events_outcome_divider_check
    CHECK (outcome_divider IS NULL OR outcome_divider > 0);
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE refdata.events ADD CONSTRAINT events_outcome_grid_check
    CHECK (
      outcome_step > 0
      AND (outcome_min IS NULL OR outcome_max IS NULL OR outcome_min <= outcome_max)
      AND (outcome_min IS NULL OR mod(outcome_min - outcome_anchor, outcome_step) = 0)
      AND (outcome_max IS NULL OR mod(outcome_max - outcome_anchor, outcome_step) = 0)
    );
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

-- CM-04: margin_model is immutable once the event has its first order, so an
-- event can never hold money on both the ISOLATED and the PORTFOLIO path.
CREATE OR REPLACE FUNCTION refdata.forbid_margin_model_change() RETURNS trigger AS $$
BEGIN
  IF NEW.margin_model IS DISTINCT FROM OLD.margin_model AND EXISTS (
       SELECT 1 FROM orders.orders o
       JOIN refdata.contracts c ON c.ticker = o.ticker
       WHERE c.event_ticker = OLD.event_ticker) THEN
    RAISE EXCEPTION 'margin_model is immutable once the event has orders (%)', OLD.event_ticker;
  END IF;
  RETURN NEW;
END $$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS events_margin_model_immutable ON refdata.events;
CREATE TRIGGER events_margin_model_immutable
  BEFORE UPDATE OF margin_model ON refdata.events
  FOR EACH ROW EXECUTE FUNCTION refdata.forbid_margin_model_change();

DO $$ BEGIN
  ALTER TABLE refdata.contracts ADD CONSTRAINT contracts_payout_ticks_check
    CHECK (payout_ticks > 0);
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE refdata.contracts ADD CONSTRAINT contracts_payoff_rule_shape_check
    CHECK (
      payoff_rule IS NULL
      OR (payoff_rule->>'type' = 'range'
          AND (payoff_rule->'lo' IS NULL OR jsonb_typeof(payoff_rule->'lo') IN ('number', 'null'))
          AND (payoff_rule->'hi' IS NULL OR jsonb_typeof(payoff_rule->'hi') IN ('number', 'null')))
      OR (payoff_rule->>'type' = 'categories'
          AND jsonb_typeof(payoff_rule->'yes_values') = 'array'
          AND jsonb_array_length(payoff_rule->'yes_values') > 0)
    );
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

-- ---------------------------------------------------------------- risk (margin authority)

-- Authoritative signed YES/LONG quantity per user and contract on PORTFOLIO
-- events. Updated in the same transaction as the margin change it causes.
CREATE TABLE IF NOT EXISTS risk.margin_positions (
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  event_ticker TEXT NOT NULL,
  net_qty BIGINT NOT NULL DEFAULT 0,
  last_global_seq BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, ticker)
);
CREATE INDEX IF NOT EXISTS idx_margin_positions_event ON risk.margin_positions(event_ticker, user_id);

-- Working orders that count toward margin, already normalised to units:
-- signed_remaining > 0 buys units, < 0 sells units, at price_ticks.
CREATE TABLE IF NOT EXISTS risk.margin_orders (
  order_id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  event_ticker TEXT NOT NULL,
  signed_remaining BIGINT NOT NULL CHECK (signed_remaining <> 0),
  price_ticks BIGINT NOT NULL,
  -- CM-02: fills and releases arrive through one outbox in global_seq order;
  -- an event with global_seq <= last_global_seq is an idempotent no-op.
  last_global_seq BIGINT NOT NULL DEFAULT 0,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_margin_orders_user_event ON risk.margin_orders(user_id, event_ticker);

-- Cached requirement per user and event, and what has been posted to
-- LIAB:USER:{user}:MARGIN:{event}. margin = posted once the journal is drained.
CREATE TABLE IF NOT EXISTS risk.event_margin (
  user_id TEXT NOT NULL,
  event_ticker TEXT NOT NULL,
  margin_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (margin_micro_usdc >= 0),
  position_requirement_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (position_requirement_micro_usdc >= 0),
  posted_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (posted_micro_usdc >= 0),
  standalone_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (standalone_micro_usdc >= 0),
  worst_outcome TEXT,
  version BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, event_ticker)
);

-- Outbox of ledger postings caused by margin changes. A reconciler replays
-- PENDING rows; the ledger idempotency key makes replays safe.
CREATE TABLE IF NOT EXISTS risk.margin_journal (
  journal_id BIGSERIAL PRIMARY KEY,
  user_id TEXT NOT NULL,
  event_ticker TEXT NOT NULL,
  cause TEXT NOT NULL CHECK (cause IN ('RESERVE', 'RELEASE', 'FILL', 'SETTLE', 'RECONCILE')),
  cause_id TEXT NOT NULL,
  before_micro_usdc BIGINT NOT NULL,
  after_micro_usdc BIGINT NOT NULL,
  ledger_idempotency_key TEXT NOT NULL UNIQUE,
  ledger_tx_id TEXT,
  status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'POSTED', 'REJECTED')),
  attempts INTEGER NOT NULL DEFAULT 0,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  posted_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_margin_journal_pending ON risk.margin_journal(status, journal_id) WHERE status = 'PENDING';

-- Makes ApplyFill idempotent: a fill is applied to margin exactly once.
CREATE TABLE IF NOT EXISTS risk.applied_fills (
  fill_id TEXT PRIMARY KEY,
  event_ticker TEXT NOT NULL,
  global_seq BIGINT NOT NULL,
  ledger_tx_id TEXT,
  applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- CM-08: shadow gate, capital compared to capital. Gate for production:
-- worst_case <= capital_cross <= capital_today on every row for two weeks.
--   capital_cross = net premium paid at actual fill prices + margin
--   capital_today = today's holds + escrow attributable to the account on the event
CREATE TABLE IF NOT EXISTS risk.margin_shadow_log (
  id BIGSERIAL PRIMARY KEY,
  user_id TEXT NOT NULL,
  event_ticker TEXT NOT NULL,
  as_of TIMESTAMPTZ NOT NULL DEFAULT now(),
  net_premium_micro_usdc BIGINT NOT NULL,
  margin_micro_usdc BIGINT NOT NULL,
  capital_cross_micro_usdc BIGINT NOT NULL,
  capital_today_micro_usdc BIGINT NOT NULL,
  worst_case_micro_usdc BIGINT NOT NULL,
  -- CM-12: STATIC_HEDGE, DELTA_HEDGE, DIRECTIONAL or MIXED
  strategy_tag TEXT,
  violation TEXT CHECK (violation IS NULL OR violation IN ('ABOVE_TODAY', 'BELOW_WORST_CASE'))
);
CREATE INDEX IF NOT EXISTS idx_margin_shadow_violation
  ON risk.margin_shadow_log(violation) WHERE violation IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_margin_shadow_user_event
  ON risk.margin_shadow_log(user_id, event_ticker, as_of);

-- ---------------------------------------------------------------- ledger

-- Held balance now includes the per-event margin accounts
-- LIAB:USER:{user}:MARGIN:{event}. Clearing accounts LIAB:HOUSE:CLEARING:{event}
-- are created lazily by ledger-svc and carry no user_id.
CREATE OR REPLACE VIEW ledger.user_balances AS
SELECT a.user_id,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:CASH' THEN latest.running_balance_micro_usdc ELSE 0 END), 0)::BIGINT AS cash_micro_usdc,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:HOLDS'
                      OR a.account_code LIKE 'LIAB:USER:%:MARGIN:%'
                    THEN latest.running_balance_micro_usdc ELSE 0 END), 0)::BIGINT AS held_micro_usdc
FROM ledger.accounts a
LEFT JOIN LATERAL (
  SELECT running_balance_micro_usdc FROM ledger.entries e
  WHERE e.account_id = a.account_id ORDER BY e.entry_id DESC LIMIT 1
) latest ON TRUE
WHERE a.user_id IS NOT NULL
GROUP BY a.user_id;

-- ---------------------------------------------------------------- settlement

-- A PORTFOLIO event settles all of its contracts in one zero-sum pass.
CREATE TABLE IF NOT EXISTS settlement.event_settlements (
  event_ticker TEXT PRIMARY KEY,
  numeric_value BIGINT,
  categorical_value TEXT,
  close_global_seq BIGINT NOT NULL DEFAULT 0,
  total_credits_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (total_credits_micro_usdc >= 0),
  total_debits_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (total_debits_micro_usdc >= 0),
  accounts_settled INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'POSTING', 'COMPLETED', 'FAILED')),
  started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  completed_at TIMESTAMPTZ,
  CHECK (status <> 'COMPLETED' OR total_credits_micro_usdc = total_debits_micro_usdc)
);

CREATE TABLE IF NOT EXISTS settlement.event_settlement_lines (
  id BIGSERIAL PRIMARY KEY,
  event_ticker TEXT NOT NULL REFERENCES settlement.event_settlements(event_ticker),
  user_id TEXT NOT NULL,
  net_micro_usdc BIGINT NOT NULL,
  margin_released_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (margin_released_micro_usdc >= 0),
  idempotency_key TEXT NOT NULL UNIQUE,
  ledger_tx_id TEXT,
  status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'POSTED', 'FAILED')),
  posted_at TIMESTAMPTZ,
  UNIQUE (event_ticker, user_id)
);
CREATE INDEX IF NOT EXISTS idx_event_settlement_lines_pending
  ON settlement.event_settlement_lines(event_ticker, status, id);
