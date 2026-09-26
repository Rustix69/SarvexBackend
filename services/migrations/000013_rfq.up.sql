CREATE SCHEMA IF NOT EXISTS rfq;

CREATE TABLE IF NOT EXISTS rfq.requests (
  rfq_id TEXT PRIMARY KEY,
  client_rfq_id TEXT NOT NULL,
  creator_user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  side INTEGER NOT NULL,
  action INTEGER NOT NULL,
  requested_count BIGINT NOT NULL CHECK (requested_count > 0),
  expires_at TIMESTAMPTZ NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('OPEN', 'ACCEPTED_PENDING_EXECUTION', 'EXECUTED', 'CANCELLED', 'EXPIRED', 'REJECTED')),
  accepted_quote_id TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (creator_user_id, client_rfq_id)
);

CREATE INDEX IF NOT EXISTS idx_rfq_requests_creator_created
  ON rfq.requests(creator_user_id, created_at DESC, rfq_id DESC);

CREATE TABLE IF NOT EXISTS rfq.quotes (
  quote_id TEXT PRIMARY KEY,
  rfq_id TEXT NOT NULL REFERENCES rfq.requests(rfq_id),
  maker_user_id TEXT NOT NULL,
  bid_price_ticks BIGINT NOT NULL CHECK (bid_price_ticks > 0),
  offer_price_ticks BIGINT NOT NULL CHECK (offer_price_ticks > 0),
  available_count BIGINT NOT NULL CHECK (available_count > 0),
  expires_at TIMESTAMPTZ NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('PENDING', 'ACCEPTED', 'REJECTED', 'CANCELLED', 'EXPIRED')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (rfq_id, maker_user_id, quote_id)
);

CREATE INDEX IF NOT EXISTS idx_rfq_quotes_request_status
  ON rfq.quotes(rfq_id, status, created_at, quote_id);
