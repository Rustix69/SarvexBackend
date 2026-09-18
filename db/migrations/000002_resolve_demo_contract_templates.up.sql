-- Resolve workbook template values for the demo catalog.
-- These are explicit demo assumptions, not oracle observations. Keep the
-- existing tickers stable so existing orders, fills, and market-data keys
-- remain valid.
BEGIN;

UPDATE refdata.contracts
SET question = 'Will Gavin Newsom be the 2028 Democratic presidential nominee?',
    settlement_rule = COALESCE(settlement_rule, '{}'::jsonb) ||
      '{"demo_assumption":{"candidate":"Gavin Newsom","source":"demo_assumption"}}'::jsonb,
    updated_at = now()
WHERE ticker = 'SX-PRESNOMD-28-{CAND}';

UPDATE refdata.contracts
SET question = 'TASI above 11,500 on 29 Oct 2026?',
    settlement_rule = COALESCE(settlement_rule, '{}'::jsonb) ||
      '{"demo_assumption":{"strike":11500,"unit":"index points","basis":"listing-day close rounded to nearest 100","source":"demo_assumption"}}'::jsonb,
    updated_at = now()
WHERE ticker = 'SX-TASI-26OCT29-ATM';

UPDATE refdata.contracts
SET question = 'Nikkei above 42,000 on 30 Oct 2026?',
    settlement_rule = COALESCE(settlement_rule, '{}'::jsonb) ||
      '{"demo_assumption":{"strike":42000,"unit":"index points","basis":"listing-day close rounded to nearest 500","source":"demo_assumption"}}'::jsonb,
    updated_at = now()
WHERE ticker = 'SX-N225-26OCT30-ATM';

UPDATE refdata.contracts
SET question = 'EU gas (TTF) above EUR 35 end-Oct?',
    settlement_rule = COALESCE(settlement_rule, '{}'::jsonb) ||
      '{"demo_assumption":{"strike":35,"unit":"EUR/MWh","basis":"listing-day settlement rounded to EUR 1","source":"demo_assumption"}}'::jsonb,
    updated_at = now()
WHERE ticker = 'SX-TTF-26OCT30-ATM';

COMMIT;
