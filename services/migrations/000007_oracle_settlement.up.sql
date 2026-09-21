CREATE SCHEMA IF NOT EXISTS oracle;
CREATE SCHEMA IF NOT EXISTS settlement;

INSERT INTO ledger.accounts (account_code, account_type, currency)
VALUES ('REVENUE:SETTLEMENT_ROUNDING', 'REVENUE'::ledger.account_type, 'USDC')
ON CONFLICT (account_code) DO NOTHING;

DO $$ BEGIN
  CREATE TYPE oracle.resolution_status AS ENUM ('PENDING', 'PROPOSED', 'FINALIZED', 'DISPUTED');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS oracle.attestations (
  id BIGSERIAL PRIMARY KEY,
  event_ticker TEXT NOT NULL,
  attestor_id TEXT NOT NULL,
  source TEXT NOT NULL,
  numeric_value BIGINT,
  categorical_value TEXT,
  signature BYTEA NOT NULL DEFAULT ''::bytea,
  observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(event_ticker, attestor_id, source),
  CHECK (numeric_value IS NOT NULL OR NULLIF(categorical_value, '') IS NOT NULL)
);
CREATE INDEX IF NOT EXISTS idx_oracle_attestations_event ON oracle.attestations(event_ticker, observed_at);

CREATE TABLE IF NOT EXISTS oracle.resolutions (
  event_ticker TEXT PRIMARY KEY,
  status oracle.resolution_status NOT NULL DEFAULT 'PENDING',
  numeric_value BIGINT,
  categorical_value TEXT,
  proposed_at TIMESTAMPTZ,
  finalized_at TIMESTAMPTZ,
  challenge_window_ends_at TIMESTAMPTZ,
  attestor_count INTEGER NOT NULL DEFAULT 0,
  required_quorum INTEGER NOT NULL DEFAULT 1,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (numeric_value IS NOT NULL OR NULLIF(categorical_value, '') IS NOT NULL OR status = 'PENDING')
);

CREATE TABLE IF NOT EXISTS oracle.oracle_keys (
  attestor_id TEXT PRIMARY KEY,
  public_key BYTEA NOT NULL,
  active BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS settlement.settlements (
  ticker TEXT PRIMARY KEY,
  event_ticker TEXT NOT NULL,
  numeric_value BIGINT,
  categorical_value TEXT,
  winner_payout_per_contract_micro_usdc BIGINT NOT NULL DEFAULT 0,
  close_global_seq BIGINT NOT NULL DEFAULT 0,
  positions_source_global_seq BIGINT NOT NULL DEFAULT 0,
  escrow_snapshot_micro_usdc BIGINT NOT NULL DEFAULT 0,
  rounding_sweep_tx_id TEXT,
  total_payout_micro_usdc BIGINT NOT NULL DEFAULT 0,
  positions_settled INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'POSTING', 'COMPLETED', 'FAILED')),
  started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  completed_at TIMESTAMPTZ,
  UNIQUE(ticker, event_ticker)
);

CREATE TABLE IF NOT EXISTS settlement.settlement_payouts (
  id BIGSERIAL PRIMARY KEY,
  ticker TEXT NOT NULL REFERENCES settlement.settlements(ticker),
  user_id TEXT NOT NULL,
  position_qty BIGINT NOT NULL,
  payout_micro_usdc BIGINT NOT NULL CHECK (payout_micro_usdc >= 0),
  ledger_tx_id TEXT,
  idempotency_key TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'POSTED', 'FAILED')),
  posted_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_settlement_payouts_pending ON settlement.settlement_payouts(ticker, status, id);
