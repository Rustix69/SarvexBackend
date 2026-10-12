DROP TABLE IF EXISTS settlement.event_settlement_lines;
DROP TABLE IF EXISTS settlement.event_settlements;

CREATE OR REPLACE VIEW ledger.user_balances AS
SELECT a.user_id,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:CASH' THEN latest.running_balance_micro_usdc ELSE 0 END), 0)::BIGINT AS cash_micro_usdc,
  COALESCE(SUM(CASE WHEN a.account_code LIKE 'LIAB:USER:%:HOLDS' THEN latest.running_balance_micro_usdc ELSE 0 END), 0)::BIGINT AS held_micro_usdc
FROM ledger.accounts a
LEFT JOIN LATERAL (
  SELECT running_balance_micro_usdc FROM ledger.entries e
  WHERE e.account_id = a.account_id ORDER BY e.entry_id DESC LIMIT 1
) latest ON TRUE
WHERE a.user_id IS NOT NULL
GROUP BY a.user_id;

DROP TABLE IF EXISTS risk.margin_shadow_log;
DROP TABLE IF EXISTS risk.applied_fills;
DROP TABLE IF EXISTS risk.margin_journal;
DROP TABLE IF EXISTS risk.event_margin;
DROP TABLE IF EXISTS risk.margin_orders;
DROP TABLE IF EXISTS risk.margin_positions;

ALTER TABLE refdata.contracts
  DROP CONSTRAINT IF EXISTS contracts_payoff_rule_shape_check,
  DROP CONSTRAINT IF EXISTS contracts_payout_ticks_check,
  DROP COLUMN IF EXISTS payout_ticks,
  DROP COLUMN IF EXISTS payoff_rule;

DROP TRIGGER IF EXISTS events_margin_model_immutable ON refdata.events;
DROP FUNCTION IF EXISTS refdata.forbid_margin_model_change();

ALTER TABLE refdata.events
  DROP CONSTRAINT IF EXISTS events_outcome_grid_check,
  DROP CONSTRAINT IF EXISTS events_outcome_divider_check,
  DROP COLUMN IF EXISTS outcome_max,
  DROP COLUMN IF EXISTS outcome_min,
  DROP COLUMN IF EXISTS outcome_anchor,
  DROP COLUMN IF EXISTS outcome_step,
  DROP COLUMN IF EXISTS outcome_divider,
  DROP COLUMN IF EXISTS trading_halted,
  DROP CONSTRAINT IF EXISTS events_outcome_kind_check,
  DROP CONSTRAINT IF EXISTS events_margin_model_check,
  DROP COLUMN IF EXISTS outcome_kind,
  DROP COLUMN IF EXISTS margin_model;
