ALTER TABLE refdata.contracts
  DROP COLUMN IF EXISTS tick_value_micro,
  DROP COLUMN IF EXISTS multiplier_micro_per_display_unit,
  DROP COLUMN IF EXISTS divider;
