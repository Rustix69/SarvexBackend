DROP INDEX IF EXISTS gateway.idx_gateway_idempotency_operation;
ALTER TABLE gateway.idempotency_records
  DROP CONSTRAINT IF EXISTS gateway_idempotency_status_check,
  DROP COLUMN IF EXISTS status,
  DROP COLUMN IF EXISTS operation_id,
  DROP COLUMN IF EXISTS updated_at,
  ALTER COLUMN response_body SET NOT NULL;

ALTER TABLE orders.orders
  DROP CONSTRAINT IF EXISTS orders_orders_quantity_lifecycle_check,
  DROP COLUMN IF EXISTS cancelled_count,
  DROP COLUMN IF EXISTS expired_count;
