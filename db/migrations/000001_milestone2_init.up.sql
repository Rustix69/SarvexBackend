CREATE SCHEMA IF NOT EXISTS refdata;
CREATE SCHEMA IF NOT EXISTS users;
CREATE SCHEMA IF NOT EXISTS ledger;
CREATE SCHEMA IF NOT EXISTS orders;
CREATE SCHEMA IF NOT EXISTS risk;
CREATE SCHEMA IF NOT EXISTS position;
CREATE SCHEMA IF NOT EXISTS oracle;
CREATE SCHEMA IF NOT EXISTS settlement;
CREATE SCHEMA IF NOT EXISTS audit;

CREATE TYPE refdata.contract_kind AS ENUM ('BINARY', 'SCALAR');
CREATE TYPE refdata.contract_state AS ENUM (
  'DRAFT', 'LISTED', 'OPEN', 'HALTED', 'CLOSED', 'RESOLVING', 'SETTLED', 'CANCELLED'
);

CREATE TABLE refdata.series (
  series_ticker TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  description TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE refdata.events (
  event_ticker TEXT PRIMARY KEY,
  series_ticker TEXT NOT NULL REFERENCES refdata.series(series_ticker),
  title TEXT NOT NULL,
  description TEXT,
  expected_resolution_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE refdata.contracts (
  ticker TEXT PRIMARY KEY,
  event_ticker TEXT NOT NULL REFERENCES refdata.events(event_ticker),
  series_ticker TEXT NOT NULL REFERENCES refdata.series(series_ticker),
  kind refdata.contract_kind NOT NULL,
  question TEXT,
  underlying TEXT,
  tick_size BIGINT NOT NULL CHECK (tick_size > 0),
  min_price_ticks BIGINT NOT NULL,
  max_price_ticks BIGINT NOT NULL,
  lower_bound_ticks BIGINT,
  upper_bound_ticks BIGINT,
  multiplier_micro_usdc BIGINT,
  max_order_size BIGINT NOT NULL,
  position_limit_per_user BIGINT NOT NULL,
  state refdata.contract_state NOT NULL DEFAULT 'DRAFT',
  listed_at TIMESTAMPTZ,
  open_at TIMESTAMPTZ,
  close_at TIMESTAMPTZ,
  expected_resolution_at TIMESTAMPTZ NOT NULL,
  settlement_source TEXT,
  oracle_policy TEXT NOT NULL DEFAULT 'ADMIN',
  settlement_rule JSONB NOT NULL DEFAULT '{}',
  close_global_seq BIGINT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_refdata_contracts_event_ticker ON refdata.contracts(event_ticker);
CREATE INDEX idx_refdata_contracts_state ON refdata.contracts(state);

CREATE TABLE refdata.contract_state_history (
  id BIGSERIAL PRIMARY KEY,
  ticker TEXT NOT NULL REFERENCES refdata.contracts(ticker),
  old_state refdata.contract_state,
  new_state refdata.contract_state NOT NULL,
  reason TEXT,
  changed_by TEXT,
  changed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_refdata_contract_state_history_ticker_changed_at
  ON refdata.contract_state_history(ticker, changed_at);

CREATE TABLE users.users (
  user_id TEXT PRIMARY KEY,
  email TEXT UNIQUE NOT NULL,
  display_name TEXT,
  password_hash TEXT NOT NULL,
  kyc_tier INT NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'ACTIVE',
  is_admin BOOLEAN NOT NULL DEFAULT false,
  is_mm BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE users.api_keys (
  key_id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users.users(user_id),
  public_key BYTEA NOT NULL,
  label TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  revoked_at TIMESTAMPTZ
);
CREATE INDEX idx_users_api_keys_user_id ON users.api_keys(user_id);

CREATE TYPE ledger.account_type AS ENUM ('ASSET', 'LIABILITY', 'EQUITY', 'REVENUE', 'EXPENSE');

CREATE TABLE ledger.accounts (
  account_id BIGSERIAL PRIMARY KEY,
  account_code TEXT NOT NULL UNIQUE,
  account_type ledger.account_type NOT NULL,
  currency TEXT NOT NULL DEFAULT 'USDC',
  user_id TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_ledger_accounts_user_id_not_null ON ledger.accounts(user_id) WHERE user_id IS NOT NULL;

CREATE TABLE ledger.transactions (
  tx_id BIGSERIAL PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  reason_code TEXT NOT NULL,
  metadata JSONB NOT NULL DEFAULT '{}',
  posted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_ledger_transactions_reason_code ON ledger.transactions(reason_code);
CREATE INDEX idx_ledger_transactions_posted_at ON ledger.transactions(posted_at);

CREATE TABLE ledger.entries (
  entry_id BIGSERIAL PRIMARY KEY,
  tx_id BIGINT NOT NULL REFERENCES ledger.transactions(tx_id),
  account_id BIGINT NOT NULL REFERENCES ledger.accounts(account_id),
  direction CHAR(2) NOT NULL CHECK (direction IN ('DR', 'CR')),
  amount_micro_usdc BIGINT NOT NULL CHECK (amount_micro_usdc > 0),
  running_balance_micro_usdc BIGINT NOT NULL,
  account_seq BIGINT NOT NULL,
  memo TEXT,
  posted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(account_id, account_seq)
);
CREATE INDEX idx_ledger_entries_tx_id ON ledger.entries(tx_id);
CREATE INDEX idx_ledger_entries_account_id_entry_id_desc ON ledger.entries(account_id, entry_id DESC);

CREATE TABLE ledger.ledger_event_outbox (
  id BIGSERIAL PRIMARY KEY,
  tx_id BIGINT NOT NULL REFERENCES ledger.transactions(tx_id),
  event_type TEXT NOT NULL,
  payload JSONB NOT NULL,
  status TEXT NOT NULL DEFAULT 'PENDING',
  attempts INT NOT NULL DEFAULT 0,
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(tx_id, event_type)
);
CREATE INDEX idx_ledger_event_outbox_status_next_attempt_at
  ON ledger.ledger_event_outbox(status, next_attempt_at);

CREATE OR REPLACE FUNCTION ledger.assert_tx_balanced()
RETURNS trigger AS $$
DECLARE
  imbalance BIGINT;
BEGIN
  SELECT
    COALESCE(SUM(CASE WHEN direction = 'DR' THEN amount_micro_usdc ELSE 0 END), 0)
    - COALESCE(SUM(CASE WHEN direction = 'CR' THEN amount_micro_usdc ELSE 0 END), 0)
  INTO imbalance
  FROM ledger.entries
  WHERE tx_id = NEW.tx_id;

  IF imbalance <> 0 THEN
    RAISE EXCEPTION 'Ledger transaction % unbalanced by %', NEW.tx_id, imbalance;
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER trg_assert_tx_balanced
  AFTER INSERT ON ledger.entries
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION ledger.assert_tx_balanced();

CREATE OR REPLACE FUNCTION ledger.block_entries_update_delete()
RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'ledger.entries is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_block_entries_update
  BEFORE UPDATE ON ledger.entries
  FOR EACH ROW EXECUTE FUNCTION ledger.block_entries_update_delete();

CREATE TRIGGER trg_block_entries_delete
  BEFORE DELETE ON ledger.entries
  FOR EACH ROW EXECUTE FUNCTION ledger.block_entries_update_delete();

CREATE TABLE ledger.holds (
  hold_id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  amount_micro_usdc BIGINT NOT NULL CHECK (amount_micro_usdc >= 0),
  committed_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (committed_micro_usdc >= 0),
  released_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (released_micro_usdc >= 0),
  reason TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE', 'CLOSED')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  closed_at TIMESTAMPTZ,
  CHECK (committed_micro_usdc + released_micro_usdc <= amount_micro_usdc)
);
CREATE INDEX idx_ledger_holds_user_id ON ledger.holds(user_id);
CREATE INDEX idx_ledger_holds_status ON ledger.holds(status);

CREATE TABLE ledger.hold_operations (
  idempotency_key TEXT PRIMARY KEY,
  hold_id TEXT NOT NULL REFERENCES ledger.holds(hold_id),
  operation_type TEXT NOT NULL CHECK (operation_type IN ('PLACE', 'RELEASE', 'COMMIT')),
  amount_micro_usdc BIGINT NOT NULL CHECK (amount_micro_usdc >= 0),
  ledger_tx_id BIGINT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_ledger_hold_operations_hold_id ON ledger.hold_operations(hold_id);

CREATE OR REPLACE VIEW ledger.user_balances AS
SELECT
  a.user_id,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:CASH' THEN e.running_balance_micro_usdc END), 0) AS cash_micro_usdc,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:HOLDS' THEN e.running_balance_micro_usdc END), 0) AS held_micro_usdc
FROM ledger.accounts a
LEFT JOIN LATERAL (
  SELECT running_balance_micro_usdc
  FROM ledger.entries
  WHERE account_id = a.account_id
  ORDER BY entry_id DESC
  LIMIT 1
) e ON true
WHERE a.user_id IS NOT NULL
GROUP BY a.user_id;

CREATE TYPE orders.order_side AS ENUM ('YES', 'NO', 'LONG', 'SHORT');
CREATE TYPE orders.order_action AS ENUM ('BUY', 'SELL');
CREATE TYPE orders.time_in_force AS ENUM ('GTC', 'IOC', 'FOK');
CREATE TYPE orders.order_status AS ENUM ('PENDING', 'OPEN', 'PARTIAL', 'FILLED', 'CANCELLED', 'REJECTED', 'EXPIRED');

CREATE TABLE orders.orders (
  order_id TEXT PRIMARY KEY,
  client_order_id TEXT,
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  side orders.order_side NOT NULL,
  action orders.order_action NOT NULL,
  price_ticks BIGINT NOT NULL,
  count BIGINT NOT NULL CHECK (count > 0),
  filled_count BIGINT NOT NULL DEFAULT 0,
  remaining_count BIGINT NOT NULL,
  tif orders.time_in_force NOT NULL,
  post_only BOOLEAN NOT NULL DEFAULT false,
  reduce_only BOOLEAN NOT NULL DEFAULT false,
  stp TEXT,
  status orders.order_status NOT NULL,
  hold_id TEXT,
  avg_fill_price_ticks BIGINT NOT NULL DEFAULT 0,
  reject_code TEXT,
  reject_reason TEXT,
  me_global_seq BIGINT,
  me_contract_seq BIGINT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ,
  UNIQUE(user_id, client_order_id)
);
CREATE INDEX idx_orders_orders_user_id_status ON orders.orders(user_id, status);
CREATE INDEX idx_orders_orders_ticker_status ON orders.orders(ticker, status);
CREATE INDEX idx_orders_orders_status_open_partial
  ON orders.orders(status) WHERE status IN ('OPEN', 'PARTIAL');

CREATE TABLE orders.fills (
  fill_id TEXT PRIMARY KEY,
  maker_order_id TEXT NOT NULL REFERENCES orders.orders(order_id),
  taker_order_id TEXT NOT NULL REFERENCES orders.orders(order_id),
  maker_user_id TEXT NOT NULL,
  taker_user_id TEXT NOT NULL,
  maker_hold_id TEXT,
  taker_hold_id TEXT,
  ticker TEXT NOT NULL,
  maker_side orders.order_side NOT NULL,
  maker_action orders.order_action NOT NULL,
  taker_side orders.order_side NOT NULL,
  taker_action orders.order_action NOT NULL,
  price_ticks BIGINT NOT NULL,
  count BIGINT NOT NULL CHECK (count > 0),
  aggressor_side orders.order_side NOT NULL,
  maker_fee_micro_usdc BIGINT NOT NULL DEFAULT 0,
  taker_fee_micro_usdc BIGINT NOT NULL DEFAULT 0,
  ticker_seq BIGINT NOT NULL,
  global_seq BIGINT NOT NULL UNIQUE,
  ledger_post_status TEXT NOT NULL DEFAULT 'PENDING',
  ledger_tx_id BIGINT,
  ts TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(ticker, ticker_seq)
);
CREATE INDEX idx_orders_fills_ticker_ts_desc ON orders.fills(ticker, ts DESC);
CREATE INDEX idx_orders_fills_maker_user_id_ts_desc ON orders.fills(maker_user_id, ts DESC);
CREATE INDEX idx_orders_fills_taker_user_id_ts_desc ON orders.fills(taker_user_id, ts DESC);
CREATE INDEX idx_orders_fills_ledger_post_status_global_seq
  ON orders.fills(ledger_post_status, global_seq);

CREATE TABLE orders.fill_posting_outbox (
  fill_id TEXT PRIMARY KEY REFERENCES orders.fills(fill_id),
  global_seq BIGINT NOT NULL UNIQUE,
  status TEXT NOT NULL DEFAULT 'PENDING',
  attempts INT NOT NULL DEFAULT 0,
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_error TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_orders_fill_posting_outbox_status_next_attempt_at
  ON orders.fill_posting_outbox(status, next_attempt_at);

CREATE TABLE risk.user_limits (
  user_id TEXT PRIMARY KEY,
  kyc_tier INT NOT NULL DEFAULT 0,
  max_order_size_micro_usdc BIGINT NOT NULL DEFAULT 100000000,
  daily_loss_limit_micro_usdc BIGINT NOT NULL DEFAULT 1000000000,
  orders_per_second_limit INT NOT NULL DEFAULT 5,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE risk.contract_position_limits (
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  max_qty BIGINT NOT NULL,
  PRIMARY KEY (user_id, ticker)
);

CREATE TABLE risk.working_orders_summary (
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  side TEXT NOT NULL,
  total_qty BIGINT NOT NULL DEFAULT 0,
  total_max_loss_micro_usdc BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, ticker, side)
);

CREATE TABLE position.positions (
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  net_qty BIGINT NOT NULL DEFAULT 0,
  avg_cost_micro_usdc BIGINT NOT NULL DEFAULT 0,
  realized_pnl_micro_usdc BIGINT NOT NULL DEFAULT 0,
  last_global_seq BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, ticker)
);
CREATE INDEX idx_position_positions_ticker_net_qty_not_zero
  ON position.positions(ticker) WHERE net_qty != 0;

CREATE TABLE position.position_history (
  id BIGSERIAL PRIMARY KEY,
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  net_qty_before BIGINT NOT NULL,
  net_qty_after BIGINT NOT NULL,
  fill_id TEXT NOT NULL,
  global_seq BIGINT NOT NULL,
  ts TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE position.applied_fills (
  fill_id TEXT PRIMARY KEY,
  ticker TEXT NOT NULL,
  global_seq BIGINT NOT NULL UNIQUE,
  applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE position.consumer_offsets (
  stream_name TEXT PRIMARY KEY,
  last_global_seq BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE position.open_interest (
  ticker TEXT PRIMARY KEY,
  total_open_long BIGINT NOT NULL DEFAULT 0,
  total_open_short BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TYPE oracle.resolution_status AS ENUM ('PENDING', 'PROPOSED', 'FINALIZED', 'DISPUTED');

CREATE TABLE oracle.attestations (
  id BIGSERIAL PRIMARY KEY,
  event_ticker TEXT NOT NULL,
  attestor_id TEXT NOT NULL,
  source TEXT NOT NULL,
  numeric_value BIGINT,
  categorical_value TEXT,
  signature BYTEA,
  observed_at TIMESTAMPTZ NOT NULL,
  received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(event_ticker, attestor_id, source)
);
CREATE INDEX idx_oracle_attestations_event_ticker ON oracle.attestations(event_ticker);

CREATE TABLE oracle.resolutions (
  event_ticker TEXT PRIMARY KEY,
  status oracle.resolution_status NOT NULL DEFAULT 'PENDING',
  numeric_value BIGINT,
  categorical_value TEXT,
  proposed_at TIMESTAMPTZ,
  finalized_at TIMESTAMPTZ,
  challenge_window_ends_at TIMESTAMPTZ,
  attestor_count INT NOT NULL DEFAULT 0,
  required_quorum INT NOT NULL DEFAULT 1,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oracle.oracle_keys (
  attestor_id TEXT PRIMARY KEY,
  public_key BYTEA NOT NULL,
  active BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE settlement.settlements (
  ticker TEXT PRIMARY KEY,
  event_ticker TEXT NOT NULL,
  numeric_value BIGINT,
  categorical_value TEXT,
  winner_payout_per_contract_micro_usdc BIGINT NOT NULL,
  close_global_seq BIGINT NOT NULL DEFAULT 0,
  positions_source_global_seq BIGINT NOT NULL DEFAULT 0,
  rounding_sweep_tx_id BIGINT,
  total_payout_micro_usdc BIGINT NOT NULL DEFAULT 0,
  positions_settled INT NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'PENDING',
  started_at TIMESTAMPTZ,
  completed_at TIMESTAMPTZ
);

CREATE TABLE settlement.settlement_payouts (
  id BIGSERIAL PRIMARY KEY,
  ticker TEXT NOT NULL REFERENCES settlement.settlements(ticker),
  user_id TEXT NOT NULL,
  position_qty BIGINT NOT NULL,
  payout_micro_usdc BIGINT NOT NULL,
  ledger_tx_id BIGINT,
  idempotency_key TEXT UNIQUE NOT NULL,
  status TEXT NOT NULL DEFAULT 'PENDING',
  posted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_settlement_payouts_ticker ON settlement.settlement_payouts(ticker);
CREATE INDEX idx_settlement_payouts_user_id ON settlement.settlement_payouts(user_id);

CREATE TABLE audit.events (
  event_id BIGSERIAL PRIMARY KEY,
  event_seq BIGINT UNIQUE NOT NULL,
  service TEXT NOT NULL,
  type TEXT NOT NULL,
  actor TEXT NOT NULL,
  subject TEXT,
  payload JSONB NOT NULL,
  prev_hash BYTEA,
  hash BYTEA,
  trace_id TEXT,
  ts TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_audit_events_service_ts_desc ON audit.events(service, ts DESC);
CREATE INDEX idx_audit_events_actor_ts_desc ON audit.events(actor, ts DESC);
CREATE INDEX idx_audit_events_subject_ts_desc ON audit.events(subject, ts DESC);
CREATE INDEX idx_audit_events_type_ts_desc ON audit.events(type, ts DESC);

CREATE SEQUENCE audit.event_seq_gen;
