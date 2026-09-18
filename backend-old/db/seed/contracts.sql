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

-- Numeric futures for the SarvaEX Futures demo section.
INSERT INTO refdata.series (series_ticker, title, description) VALUES
  ('SARVEX-FUTURES', 'SarvaEX Numeric Futures', 'Demo scalar event futures on macro, crypto, rates, and market levels')
ON CONFLICT (series_ticker) DO NOTHING;

INSERT INTO refdata.events (event_ticker, series_ticker, title, description, expected_resolution_at) VALUES
  ('FUT-INDIA-GDP-FY26', 'SARVEX-FUTURES', 'India GDP FY2026', 'Expected India nominal GDP in USD trillions', '2026-12-31 12:00:00+00'),
  ('FUT-BTC-JUN26', 'SARVEX-FUTURES', 'Bitcoin June 2026 Level', 'Expected Bitcoin USD level at June 2026 close', '2026-06-30 23:59:00+00'),
  ('FUT-ETH-JUN26', 'SARVEX-FUTURES', 'Ethereum June 2026 Level', 'Expected Ethereum USD level at June 2026 close', '2026-06-30 23:59:00+00'),
  ('FUT-AI-MCAP-DEC26', 'SARVEX-FUTURES', 'AI Market Cap 2026', 'Expected public AI market cap in USD trillions', '2026-12-31 23:59:00+00'),
  ('FUT-INDIA-UNEMP-DEC26', 'SARVEX-FUTURES', 'India Unemployment Dec 2026', 'Expected India unemployment rate percentage', '2026-12-31 12:00:00+00'),
  ('FUT-USDINR-DEC26', 'SARVEX-FUTURES', 'USD/INR Year-End 2026', 'Expected USD/INR level at year end', '2026-12-31 23:59:00+00'),
  ('FUT-NIFTY-DEC26', 'SARVEX-FUTURES', 'Nifty Year-End 2026', 'Expected Nifty 50 index level at year end', '2026-12-31 23:59:00+00'),
  ('FUT-FEDRATE-DEC26', 'SARVEX-FUTURES', 'US Fed Rate Dec 2026', 'Expected US Fed target rate percentage', '2026-12-16 20:00:00+00')
ON CONFLICT (event_ticker) DO NOTHING;

INSERT INTO refdata.contracts (
  ticker, event_ticker, series_ticker, kind, underlying,
  tick_size, min_price_ticks, max_price_ticks,
  lower_bound_ticks, upper_bound_ticks, multiplier_micro_usdc,
  max_order_size, position_limit_per_user, state,
  open_at, close_at, expected_resolution_at,
  settlement_source, oracle_policy, settlement_rule
) VALUES
  ('FUT-INDIA-GDP-FY26-SCALAR', 'FUT-INDIA-GDP-FY26', 'SARVEX-FUTURES', 'SCALAR', 'India nominal GDP in USD trillions', 1, 250, 600, 250, 600, 1000000, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-12-31 11:00:00+00', '2026-12-31 12:00:00+00', 'https://mospi.gov.in', 'ADMIN', '{"type":"scalar_numeric"}'::jsonb),
  ('FUT-BTC-JUN26-LEVEL', 'FUT-BTC-JUN26', 'SARVEX-FUTURES', 'SCALAR', 'Bitcoin USD price', 500, 70000, 180000, 70000, 180000, 1000, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-06-30 23:00:00+00', '2026-06-30 23:59:00+00', 'https://coinbase.com', 'ADMIN', '{"type":"scalar_numeric"}'::jsonb),
  ('FUT-ETH-JUN26-LEVEL', 'FUT-ETH-JUN26', 'SARVEX-FUTURES', 'SCALAR', 'Ethereum USD price', 50, 3000, 12000, 3000, 12000, 10000, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-06-30 23:00:00+00', '2026-06-30 23:59:00+00', 'https://coinbase.com', 'ADMIN', '{"type":"scalar_numeric"}'::jsonb),
  ('FUT-AI-MCAP-DEC26-SCALAR', 'FUT-AI-MCAP-DEC26', 'SARVEX-FUTURES', 'SCALAR', 'Public AI market cap in USD trillions', 1, 50, 300, 50, 300, 1000000, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-12-31 22:00:00+00', '2026-12-31 23:59:00+00', 'https://sec.gov', 'ADMIN', '{"type":"scalar_numeric"}'::jsonb),
  ('FUT-INDIA-UNEMP-DEC26-SCALAR', 'FUT-INDIA-UNEMP-DEC26', 'SARVEX-FUTURES', 'SCALAR', 'India unemployment rate percentage', 1, 300, 1200, 300, 1200, 1000000, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-12-31 11:00:00+00', '2026-12-31 12:00:00+00', 'https://mospi.gov.in', 'ADMIN', '{"type":"scalar_numeric"}'::jsonb),
  ('FUT-USDINR-DEC26-SCALAR', 'FUT-USDINR-DEC26', 'SARVEX-FUTURES', 'SCALAR', 'USD/INR exchange rate', 5, 7000, 9500, 7000, 9500, 10000, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-12-31 22:00:00+00', '2026-12-31 23:59:00+00', 'https://rbi.org.in', 'ADMIN', '{"type":"scalar_numeric"}'::jsonb),
  ('FUT-NIFTY-DEC26-LEVEL', 'FUT-NIFTY-DEC26', 'SARVEX-FUTURES', 'SCALAR', 'Nifty 50 index level', 50, 18000, 32000, 18000, 32000, 1000, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-12-31 22:00:00+00', '2026-12-31 23:59:00+00', 'https://nseindia.com', 'ADMIN', '{"type":"scalar_numeric"}'::jsonb),
  ('FUT-FEDRATE-DEC26-SCALAR', 'FUT-FEDRATE-DEC26', 'SARVEX-FUTURES', 'SCALAR', 'US Fed target rate percentage', 1, 200, 650, 200, 650, 1000000, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-12-16 19:00:00+00', '2026-12-16 20:00:00+00', 'https://federalreserve.gov', 'ADMIN', '{"type":"scalar_numeric"}'::jsonb)
ON CONFLICT (ticker) DO NOTHING;

-- Extra investor-demo markets for the frontend dashboard.
INSERT INTO refdata.series (series_ticker, title, description) VALUES
  ('SARVEX-DEMO', 'SarvaEX Demo Markets', 'Demo binary markets for investor frontend walkthroughs')
ON CONFLICT (series_ticker) DO NOTHING;

INSERT INTO refdata.events (event_ticker, series_ticker, title, description, expected_resolution_at) VALUES
  ('DEMO-FED-JUL26', 'SARVEX-DEMO', 'Fed July 2026 Decision', 'Demo market for the July 2026 FOMC decision', '2026-07-29 18:00:00+00'),
  ('DEMO-BTC-JUN26', 'SARVEX-DEMO', 'Bitcoin June 2026 Price', 'Demo market for BTC price action', '2026-06-30 23:59:00+00'),
  ('DEMO-ETH-JUN26', 'SARVEX-DEMO', 'Ethereum June 2026 Price', 'Demo market for ETH price action', '2026-06-30 23:59:00+00'),
  ('DEMO-OIL-MAY26', 'SARVEX-DEMO', 'WTI Oil May 2026', 'Demo market for WTI crude oil', '2026-05-31 23:59:00+00'),
  ('DEMO-AI-DEC26', 'SARVEX-DEMO', 'AI Revenue 2026', 'Demo market for AI revenue outcomes', '2026-12-31 23:59:00+00'),
  ('DEMO-WC-2026', 'SARVEX-DEMO', '2026 World Cup', 'Demo market for World Cup outcomes', '2026-07-19 23:59:00+00'),
  ('DEMO-US-HOUSE-2026', 'SARVEX-DEMO', 'US House 2026', 'Demo market for US House control', '2026-11-04 06:00:00+00'),
  ('DEMO-TESLA-Q2-26', 'SARVEX-DEMO', 'Tesla Q2 2026 Deliveries', 'Demo market for Tesla deliveries', '2026-07-02 13:00:00+00'),
  ('DEMO-NVIDIA-AUG26', 'SARVEX-DEMO', 'NVIDIA August 2026 Earnings', 'Demo market for NVIDIA earnings', '2026-08-28 21:00:00+00'),
  ('DEMO-INDIA-GDP-Q2-26', 'SARVEX-DEMO', 'India GDP Q2 2026', 'Demo market for India GDP growth', '2026-08-31 12:00:00+00')
ON CONFLICT (event_ticker) DO NOTHING;

INSERT INTO refdata.contracts (
  ticker, event_ticker, series_ticker, kind, question,
  tick_size, min_price_ticks, max_price_ticks,
  max_order_size, position_limit_per_user, state,
  open_at, close_at, expected_resolution_at,
  settlement_source, oracle_policy, settlement_rule
) VALUES
  ('DEMO-FED-JUL26-CUT', 'DEMO-FED-JUL26', 'SARVEX-DEMO', 'BINARY', 'Will the Fed cut rates at the July 2026 meeting?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-07-29 16:00:00+00', '2026-07-29 18:00:00+00', 'https://federalreserve.gov', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb),
  ('DEMO-BTC-JUN26-120K', 'DEMO-BTC-JUN26', 'SARVEX-DEMO', 'BINARY', 'Will Bitcoin trade above $120k by June 30, 2026?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-06-30 23:00:00+00', '2026-06-30 23:59:00+00', 'https://coinbase.com', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb),
  ('DEMO-ETH-JUN26-8K', 'DEMO-ETH-JUN26', 'SARVEX-DEMO', 'BINARY', 'Will Ethereum trade above $8k by June 30, 2026?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-06-30 23:00:00+00', '2026-06-30 23:59:00+00', 'https://coinbase.com', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb),
  ('DEMO-OIL-MAY26-95', 'DEMO-OIL-MAY26', 'SARVEX-DEMO', 'BINARY', 'Will WTI crude oil close above $95 in May 2026?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-05-31 21:00:00+00', '2026-05-31 23:59:00+00', 'https://eia.gov', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb),
  ('DEMO-AI-DEC26-1T', 'DEMO-AI-DEC26', 'SARVEX-DEMO', 'BINARY', 'Will public AI company revenue exceed $1T in 2026?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-12-31 22:00:00+00', '2026-12-31 23:59:00+00', 'https://sec.gov', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb),
  ('DEMO-WC-2026-FRANCE', 'DEMO-WC-2026', 'SARVEX-DEMO', 'BINARY', 'Will France win the 2026 FIFA World Cup?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-07-19 18:00:00+00', '2026-07-19 23:59:00+00', 'https://fifa.com', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb),
  ('DEMO-US-HOUSE-2026-DEM', 'DEMO-US-HOUSE-2026', 'SARVEX-DEMO', 'BINARY', 'Will Democrats win control of the House in 2026?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-11-04 05:00:00+00', '2026-11-04 06:00:00+00', 'https://fec.gov', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb),
  ('DEMO-TESLA-Q2-26-500K', 'DEMO-TESLA-Q2-26', 'SARVEX-DEMO', 'BINARY', 'Will Tesla deliver over 500k vehicles in Q2 2026?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-07-02 12:00:00+00', '2026-07-02 13:00:00+00', 'https://ir.tesla.com', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb),
  ('DEMO-NVIDIA-AUG26-5T', 'DEMO-NVIDIA-AUG26', 'SARVEX-DEMO', 'BINARY', 'Will NVIDIA close above $5T market cap after August earnings?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-08-28 20:00:00+00', '2026-08-28 21:00:00+00', 'https://investor.nvidia.com', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb),
  ('DEMO-INDIA-GDP-Q2-26-7PCT', 'DEMO-INDIA-GDP-Q2-26', 'SARVEX-DEMO', 'BINARY', 'Will India Q2 2026 GDP growth exceed 7%?', 1, 1, 99, 100000, 250000, 'OPEN', '2026-04-01 00:00:00+00', '2026-08-31 11:00:00+00', '2026-08-31 12:00:00+00', 'https://mospi.gov.in', 'ADMIN', '{"type":"categorical_equals","yes_values":["YES"]}'::jsonb)
ON CONFLICT (ticker) DO NOTHING;
