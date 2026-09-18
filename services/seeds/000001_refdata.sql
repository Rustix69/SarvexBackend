BEGIN;

INSERT INTO refdata.series (series_ticker, title, description)
VALUES
  ('XLSX-STANDARD', 'Standard demo contracts', 'Deterministic Sarvex Phase 01 devnet catalog'),
  ('XLSX-REGIONAL', 'Regional demo contracts', 'Deterministic Sarvex Phase 01 devnet catalog')
ON CONFLICT (series_ticker) DO NOTHING;

INSERT INTO refdata.events (event_ticker, series_ticker, title, description, expected_resolution_at)
VALUES
  ('XEV-STD-ECO-01', 'XLSX-STANDARD', 'FOMC October 2026', 'Demo economic event', '2026-10-29 00:00:00+00'),
  ('XEV-REG-ME-05', 'XLSX-REGIONAL', 'TASI October 2026', 'Demo regional equity event', '2026-10-30 00:00:00+00'),
  ('XEV-REG-AS-03', 'XLSX-REGIONAL', 'Nikkei October 2026', 'Demo regional equity event', '2026-10-31 00:00:00+00'),
  ('XEV-REG-EU-03', 'XLSX-REGIONAL', 'TTF October 2026', 'Demo energy event', '2026-10-31 00:00:00+00'),
  ('XEV-STD-ELE-03', 'XLSX-STANDARD', 'US Democratic nominee 2028', 'Demo election event', '2028-08-01 00:00:00+00')
ON CONFLICT (event_ticker) DO NOTHING;

INSERT INTO refdata.contracts (
  ticker, event_ticker, series_ticker, kind, question, underlying, tick_size,
  min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks,
  multiplier_micro_usdc, max_order_size, position_limit_per_user, state,
  listed_at, open_at, close_at, expected_resolution_at, settlement_source,
  oracle_policy, settlement_rule
)
VALUES
  ('SX-FEDDEC-26OCT-H25', 'XEV-STD-ECO-01', 'XLSX-STANDARD', 'BINARY', 'Will the FOMC raise the federal funds target range by exactly 25 bp at its 27-28 Oct 2026 meeting?', NULL, 1, 1, 99, 1, 99, NULL, 100000, 250000, 'OPEN', '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '2026-10-28 23:00:00+00', '2026-10-29 00:00:00+00', 'DEMO', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'),
  ('SX-TASI-26OCT29-ATM', 'XEV-REG-ME-05', 'XLSX-REGIONAL', 'BINARY', 'TASI above 11,500 on 29 Oct 2026?', NULL, 1, 1, 99, 1, 99, NULL, 100000, 250000, 'OPEN', '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '2026-10-29 23:00:00+00', '2026-10-30 00:00:00+00', 'DEMO', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"],"demo_assumption":{"strike":11500}}'),
  ('SX-N225-26OCT30-ATM', 'XEV-REG-AS-03', 'XLSX-REGIONAL', 'BINARY', 'Nikkei above 42,000 on 30 Oct 2026?', NULL, 1, 1, 99, 1, 99, NULL, 100000, 250000, 'OPEN', '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '2026-10-30 23:00:00+00', '2026-10-31 00:00:00+00', 'DEMO', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"],"demo_assumption":{"strike":42000}}'),
  ('SX-TTF-26OCT30-ATM', 'XEV-REG-EU-03', 'XLSX-REGIONAL', 'BINARY', 'EU gas (TTF) above EUR 35 end-Oct?', NULL, 1, 1, 99, 1, 99, NULL, 100000, 250000, 'OPEN', '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '2026-10-30 23:00:00+00', '2026-10-31 00:00:00+00', 'DEMO', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"],"demo_assumption":{"strike":35,"unit":"EUR/MWh"}}'),
  ('SX-PRESNOMD-28-{CAND}', 'XEV-STD-ELE-03', 'XLSX-STANDARD', 'BINARY', 'Will Gavin Newsom be the 2028 Democratic presidential nominee?', NULL, 1, 1, 99, 1, 99, NULL, 100000, 250000, 'OPEN', '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '2028-07-31 23:00:00+00', '2028-08-01 00:00:00+00', 'DEMO', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"],"demo_assumption":{"candidate":"Gavin Newsom"}}')
ON CONFLICT (ticker) DO UPDATE SET
  question = EXCLUDED.question,
  settlement_rule = EXCLUDED.settlement_rule,
  state = EXCLUDED.state,
  updated_at = now();

COMMIT;
