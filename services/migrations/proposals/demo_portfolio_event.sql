-- Demo cross-margin event, listed as PORTFOLIO from the start (margin_model is immutable once an event has orders).
-- Cloned from the fed-funds Dec-26 pair XEV-STD-ECO-02 (strike 4.00, outcome ticks = % x 100, 25 bp step).
BEGIN;
INSERT INTO refdata.events (event_ticker, series_ticker, title, description, expected_resolution_at, margin_model, outcome_kind, outcome_divider, outcome_step, outcome_anchor)
SELECT 'XEV-DEMO-XM-01', series_ticker, 'Cross-margin demo: fed funds upper bound above 4.00% after Dec 2026 FOMC', 'Demo PORTFOLIO event (binary + futures, cross margined).', expected_resolution_at, 'PORTFOLIO', outcome_kind, outcome_divider, outcome_step, outcome_anchor
FROM refdata.events WHERE event_ticker='XEV-STD-ECO-02';

INSERT INTO refdata.contracts (ticker, event_ticker, series_ticker, kind, question, underlying, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, multiplier_micro_usdc, max_order_size, position_limit_per_user, state, listed_at, open_at, close_at, expected_resolution_at, settlement_source, oracle_policy, settlement_rule, divider, multiplier_micro_per_display_unit, tick_value_micro, payoff_rule, payout_ticks)
SELECT CASE kind WHEN 'BINARY' THEN 'SX-XMDEMO-T4.00' ELSE 'SXF-XMDEMO' END, 'XEV-DEMO-XM-01', series_ticker, kind, question, underlying, tick_size, min_price_ticks, max_price_ticks, lower_bound_ticks, upper_bound_ticks, multiplier_micro_usdc, max_order_size, position_limit_per_user, 'OPEN', now(), now(), close_at, expected_resolution_at, settlement_source, oracle_policy, settlement_rule, divider, multiplier_micro_per_display_unit, tick_value_micro, payoff_rule, payout_ticks
FROM refdata.contracts WHERE event_ticker='XEV-STD-ECO-02';
COMMIT;
