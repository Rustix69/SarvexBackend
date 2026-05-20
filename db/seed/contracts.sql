INSERT INTO refdata.series (series_ticker, title, description) VALUES
  ('RBI-RATEDECISION', 'RBI Repo Rate Decisions',
   'Binary contracts on RBI Monetary Policy Committee rate decisions'),
  ('INDIA-CPI', 'India CPI YoY Inflation',
   'Scalar futures on India MoSPI Consumer Price Inflation YoY readings')
ON CONFLICT (series_ticker) DO NOTHING;

INSERT INTO refdata.events (event_ticker, series_ticker, title, description, expected_resolution_at) VALUES
  ('RBI-JUN26', 'RBI-RATEDECISION', 'RBI June 2026 MPC Meeting',
   'Outcome of the June 2026 RBI Monetary Policy Committee meeting',
   '2026-06-06 11:00:00+00'),
  ('INDIA-CPI-JUN26', 'INDIA-CPI', 'India CPI June 2026 Print',
   'India CPI-Combined YoY % for June 2026, as released by MoSPI',
   '2026-07-12 12:30:00+00')
ON CONFLICT (event_ticker) DO NOTHING;

INSERT INTO refdata.contracts (
  ticker, event_ticker, series_ticker, kind, question,
  tick_size, min_price_ticks, max_price_ticks,
  max_order_size, position_limit_per_user, state,
  open_at, close_at, expected_resolution_at,
  settlement_source, oracle_policy, settlement_rule
) VALUES (
  'RBI-JUN26-CUT25', 'RBI-JUN26', 'RBI-RATEDECISION', 'BINARY',
  'Will the RBI cut repo rate by 25 bps or more at the June 2026 MPC?',
  1, 1, 99,
  100000, 250000, 'OPEN',
  '2026-04-01 00:00:00+00', '2026-06-06 08:30:00+00', '2026-06-06 11:00:00+00',
  'https://rbi.org.in/Scripts/BS_PressReleaseDisplay.aspx', 'MULTI_SOURCE_ATTEST',
  '{"type":"categorical_equals","yes_values":["CUT_25","CUT_50","CUT_75","CUT_100"]}'::jsonb
)
ON CONFLICT (ticker) DO NOTHING;

INSERT INTO refdata.contracts (
  ticker, event_ticker, series_ticker, kind, underlying,
  tick_size, min_price_ticks, max_price_ticks,
  lower_bound_ticks, upper_bound_ticks, multiplier_micro_usdc,
  max_order_size, position_limit_per_user, state,
  open_at, close_at, expected_resolution_at,
  settlement_source, oracle_policy, settlement_rule
) VALUES (
  'INDIA-CPI-JUN26-SCALAR', 'INDIA-CPI-JUN26', 'INDIA-CPI', 'SCALAR',
  'India CPI-C YoY % (basis points)',
  1, 200, 800,
  200, 800, 1000000,
  100000, 250000, 'OPEN',
  '2026-04-01 00:00:00+00', '2026-07-12 12:00:00+00', '2026-07-12 12:30:00+00',
  'https://mospi.gov.in/cpi', 'SINGLE_SOURCE',
  '{"type":"scalar_numeric"}'::jsonb
)
ON CONFLICT (ticker) DO NOTHING;
