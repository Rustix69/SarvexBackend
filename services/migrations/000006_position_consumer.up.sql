ALTER TABLE position.positions
  ADD COLUMN IF NOT EXISTS avg_cost_micro_usdc BIGINT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS realized_pnl_micro_usdc BIGINT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS unrealized_pnl_micro_usdc BIGINT NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS position.consumer_offsets (
  consumer_name TEXT PRIMARY KEY,
  stream_name TEXT NOT NULL,
  last_global_seq BIGINT NOT NULL DEFAULT 0 CHECK (last_global_seq >= 0),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS position.applied_fills (
  fill_id TEXT PRIMARY KEY,
  ticker TEXT NOT NULL,
  global_seq BIGINT NOT NULL UNIQUE,
  applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_position_applied_fills_seq
  ON position.applied_fills(global_seq);
