ALTER TABLE orders.orders
  ADD COLUMN IF NOT EXISTS opening_qty_reserved BIGINT NOT NULL DEFAULT 0 CHECK (opening_qty_reserved >= 0),
  ADD COLUMN IF NOT EXISTS closing_qty_reserved BIGINT NOT NULL DEFAULT 0 CHECK (closing_qty_reserved >= 0);

ALTER TABLE orders.orders
  DROP CONSTRAINT IF EXISTS orders_qty_reservation_check;

ALTER TABLE orders.orders
  ADD CONSTRAINT orders_qty_reservation_check CHECK (opening_qty_reserved + closing_qty_reserved <= count);
