CREATE TABLE IF NOT EXISTS orders.execution_event_outbox (
  event_id TEXT PRIMARY KEY,
  subject TEXT NOT NULL,
  event_type TEXT NOT NULL,
  global_seq BIGINT NOT NULL,
  contract_seq BIGINT,
  payload JSONB NOT NULL,
  status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'POSTED', 'FAILED')),
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  posted_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_execution_outbox_global_event
  ON orders.execution_event_outbox(global_seq, event_type);
CREATE INDEX IF NOT EXISTS idx_execution_outbox_pending
  ON orders.execution_event_outbox(status, next_attempt_at, global_seq);
