ALTER TABLE orders.orders
  DROP CONSTRAINT IF EXISTS orders_qty_reservation_check;

ALTER TABLE orders.orders
  DROP COLUMN IF EXISTS opening_qty_reserved,
  DROP COLUMN IF EXISTS closing_qty_reserved;
