#!/bin/sh

set -eu

run_sql() {
  psql -h postgres -U sarvex -d sarvex -v ON_ERROR_STOP=1 -f "$1"
}

run_sql /migrations/000001_refdata.up.sql
run_sql /migrations/000002_ledger.up.sql
run_sql /migrations/000003_risk.up.sql
run_sql /migrations/000004_orders.up.sql
run_sql /migrations/000005_execution_events.up.sql
run_sql /migrations/000006_position_consumer.up.sql
run_sql /migrations/000007_oracle_settlement.up.sql
run_sql /migrations/000008_gateway_idempotency.up.sql
run_sql /seeds/000001_refdata.sql
run_sql /seeds/000002_risk.sql
run_sql /seeds/000003_trade_bots.sql
