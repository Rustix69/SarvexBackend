-- Restore the original workbook templates. This rollback is for local/demo
-- environments only; do not use it after real orders reference the resolved
-- contract descriptions.
BEGIN;

UPDATE refdata.contracts
SET question = 'Will <candidate> be the 2028 Democratic presidential nominee? (one binary per candidate)',
    settlement_rule = settlement_rule - 'demo_assumption',
    updated_at = now()
WHERE ticker = 'SX-PRESNOMD-28-{CAND}';

UPDATE refdata.contracts
SET question = 'TASI above <K> on 29 Oct?',
    settlement_rule = settlement_rule - 'demo_assumption',
    updated_at = now()
WHERE ticker = 'SX-TASI-26OCT29-ATM';

UPDATE refdata.contracts
SET question = 'Nikkei above <K> on 30 Oct?',
    settlement_rule = settlement_rule - 'demo_assumption',
    updated_at = now()
WHERE ticker = 'SX-N225-26OCT30-ATM';

UPDATE refdata.contracts
SET question = 'EU gas (TTF) above <K> end-Oct?',
    settlement_rule = settlement_rule - 'demo_assumption',
    updated_at = now()
WHERE ticker = 'SX-TTF-26OCT30-ATM';

COMMIT;
