DROP TABLE IF EXISTS position.applied_fills;
DROP TABLE IF EXISTS position.consumer_offsets;
ALTER TABLE position.positions
  DROP COLUMN IF EXISTS avg_cost_micro_usdc,
  DROP COLUMN IF EXISTS realized_pnl_micro_usdc,
  DROP COLUMN IF EXISTS unrealized_pnl_micro_usdc;
