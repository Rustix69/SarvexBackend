CREATE SCHEMA IF NOT EXISTS risk;
CREATE SCHEMA IF NOT EXISTS position;

CREATE TABLE IF NOT EXISTS risk.user_limits (
  user_id TEXT PRIMARY KEY,
  kyc_tier INTEGER NOT NULL DEFAULT 0,
  max_order_size_micro_usdc BIGINT NOT NULL DEFAULT 10000000000 CHECK (max_order_size_micro_usdc > 0),
  daily_loss_limit_micro_usdc BIGINT NOT NULL DEFAULT 100000000000 CHECK (daily_loss_limit_micro_usdc > 0),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS risk.contract_position_limits (
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  max_qty BIGINT NOT NULL CHECK (max_qty > 0),
  PRIMARY KEY (user_id, ticker)
);

CREATE TABLE IF NOT EXISTS risk.working_orders_summary (
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  side TEXT NOT NULL,
  total_qty BIGINT NOT NULL DEFAULT 0 CHECK (total_qty >= 0),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, ticker, side)
);

CREATE TABLE IF NOT EXISTS position.positions (
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  net_qty BIGINT NOT NULL DEFAULT 0,
  last_global_seq BIGINT NOT NULL DEFAULT 0,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, ticker)
);
