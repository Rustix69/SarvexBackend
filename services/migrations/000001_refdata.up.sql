CREATE SCHEMA IF NOT EXISTS refdata;

DO $$ BEGIN
  CREATE TYPE refdata.contract_kind AS ENUM ('BINARY', 'SCALAR');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  CREATE TYPE refdata.contract_state AS ENUM ('DRAFT', 'LISTED', 'OPEN', 'HALTED', 'CLOSED', 'RESOLVING', 'SETTLED', 'CANCELLED');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS refdata.series (
  series_ticker TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  description TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS refdata.events (
  event_ticker TEXT PRIMARY KEY,
  series_ticker TEXT NOT NULL REFERENCES refdata.series(series_ticker),
  title TEXT NOT NULL,
  description TEXT,
  expected_resolution_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS refdata.contracts (
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

CREATE INDEX IF NOT EXISTS idx_refdata_contracts_state ON refdata.contracts(state);
CREATE INDEX IF NOT EXISTS idx_refdata_contracts_series ON refdata.contracts(series_ticker);

CREATE TABLE IF NOT EXISTS refdata.contract_state_history (
  id BIGSERIAL PRIMARY KEY,
  ticker TEXT NOT NULL REFERENCES refdata.contracts(ticker),
  old_state refdata.contract_state,
  new_state refdata.contract_state NOT NULL,
  reason TEXT,
  changed_by TEXT,
  changed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_refdata_state_history_ticker ON refdata.contract_state_history(ticker, changed_at);
