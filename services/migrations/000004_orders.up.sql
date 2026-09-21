CREATE SCHEMA IF NOT EXISTS orders;

CREATE TABLE IF NOT EXISTS orders.orders (
  order_id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  client_order_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  side INTEGER NOT NULL,
  action INTEGER NOT NULL,
  price_ticks BIGINT NOT NULL CHECK (price_ticks > 0),
  count BIGINT NOT NULL CHECK (count > 0),
  filled_count BIGINT NOT NULL DEFAULT 0 CHECK (filled_count >= 0),
  tif INTEGER NOT NULL,
  post_only BOOLEAN NOT NULL DEFAULT FALSE,
  reduce_only BOOLEAN NOT NULL DEFAULT FALSE,
  stp INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL CHECK (status IN ('PENDING', 'OPEN', 'PARTIAL', 'FILLED', 'CANCELLED', 'REJECTED', 'EXPIRED')),
  reject_code TEXT,
  hold_id TEXT,
  hold_amount_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (hold_amount_micro_usdc >= 0),
  avg_fill_price_ticks BIGINT NOT NULL DEFAULT 0 CHECK (avg_fill_price_ticks >= 0),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at TIMESTAMPTZ,
  UNIQUE(user_id, client_order_id),
  CHECK (filled_count <= count)
);

CREATE INDEX IF NOT EXISTS idx_orders_user_created
  ON orders.orders(user_id, created_at DESC, order_id DESC);
CREATE INDEX IF NOT EXISTS idx_orders_ticker_status
  ON orders.orders(ticker, status, created_at DESC);

CREATE TABLE IF NOT EXISTS orders.fills (
  fill_id TEXT PRIMARY KEY,
  ticker TEXT NOT NULL,
  global_seq BIGINT NOT NULL UNIQUE,
  contract_seq BIGINT NOT NULL,
  maker_order_id TEXT NOT NULL REFERENCES orders.orders(order_id),
  taker_order_id TEXT NOT NULL REFERENCES orders.orders(order_id),
  maker_user_id TEXT NOT NULL,
  taker_user_id TEXT NOT NULL,
  maker_hold_id TEXT,
  taker_hold_id TEXT,
  maker_side INTEGER NOT NULL,
  maker_action INTEGER NOT NULL,
  taker_side INTEGER NOT NULL,
  taker_action INTEGER NOT NULL,
  price_ticks BIGINT NOT NULL CHECK (price_ticks > 0),
  count BIGINT NOT NULL CHECK (count > 0),
  aggressor_side INTEGER NOT NULL,
  maker_fee_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (maker_fee_micro_usdc >= 0),
  taker_fee_micro_usdc BIGINT NOT NULL DEFAULT 0 CHECK (taker_fee_micro_usdc >= 0),
  posted BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS orders.fill_posting_outbox (
  outbox_id BIGSERIAL PRIMARY KEY,
  fill_id TEXT NOT NULL UNIQUE REFERENCES orders.fills(fill_id),
  status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'POSTED', 'FAILED')),
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  posted_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_fill_outbox_pending
  ON orders.fill_posting_outbox(status, next_attempt_at, outbox_id);
