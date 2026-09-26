ALTER TABLE orders.orders
  ADD COLUMN IF NOT EXISTS cancelled_count BIGINT NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS expired_count BIGINT NOT NULL DEFAULT 0;

UPDATE orders.orders
SET cancelled_count = CASE
      WHEN status = 'CANCELLED' THEN GREATEST(count - filled_count, 0)
      ELSE cancelled_count
    END,
    expired_count = CASE
      WHEN status = 'EXPIRED' THEN GREATEST(count - filled_count, 0)
      ELSE expired_count
    END
WHERE status IN ('CANCELLED', 'EXPIRED');

ALTER TABLE orders.orders
  DROP CONSTRAINT IF EXISTS orders_orders_quantity_lifecycle_check,
  ADD CONSTRAINT orders_orders_quantity_lifecycle_check
    CHECK (filled_count >= 0 AND cancelled_count >= 0 AND expired_count >= 0
           AND filled_count + cancelled_count + expired_count <= count);

ALTER TABLE gateway.idempotency_records
  ADD COLUMN IF NOT EXISTS operation_id TEXT,
  ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'COMPLETED',
  ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

UPDATE gateway.idempotency_records
SET operation_id = COALESCE(operation_id, 'op_' || md5(user_id || ':' || method || ':' || request_path || ':' || idempotency_key))
WHERE operation_id IS NULL;

ALTER TABLE gateway.idempotency_records
  ALTER COLUMN response_body DROP NOT NULL,
  ADD CONSTRAINT gateway_idempotency_status_check
    CHECK (status IN ('IN_PROGRESS', 'COMPLETED', 'UNKNOWN'));

CREATE UNIQUE INDEX IF NOT EXISTS idx_gateway_idempotency_operation
  ON gateway.idempotency_records(operation_id);
