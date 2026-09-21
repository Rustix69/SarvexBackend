CREATE SCHEMA IF NOT EXISTS gateway;

CREATE TABLE IF NOT EXISTS gateway.idempotency_records (
  user_id TEXT NOT NULL,
  method TEXT NOT NULL,
  request_path TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  request_hash TEXT NOT NULL,
  response_status INTEGER NOT NULL,
  response_body JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '24 hours'),
  PRIMARY KEY (user_id, method, request_path, idempotency_key)
);
CREATE INDEX IF NOT EXISTS idx_gateway_idempotency_expiry ON gateway.idempotency_records(expires_at);

