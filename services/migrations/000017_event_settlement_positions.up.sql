-- Snapshot of the positions an event was settled from, so a settlement that is
-- interrupted part-way is resumed from exactly the same numbers (cross margin,
-- SettleEvent). Additive only.
CREATE TABLE IF NOT EXISTS settlement.event_settlement_positions (
  event_ticker TEXT NOT NULL,
  user_id TEXT NOT NULL,
  ticker TEXT NOT NULL,
  net_qty BIGINT NOT NULL,
  PRIMARY KEY (event_ticker, user_id, ticker)
);
