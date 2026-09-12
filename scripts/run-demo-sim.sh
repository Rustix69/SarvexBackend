#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

: "${GW_REST_URL:=http://localhost:18080}"
: "${LEDGER_ADDR:=localhost:50062}"
: "${MATCHING_ADDR:=localhost:50064}"
: "${POSTGRES_DSN:=postgres://sarvaex:sarvaex@localhost:15432/sarvaex?sslmode=disable}"
: "${DEMO_TICKER:=SX-FEDDEC-26OCT-H25}"
: "${SIM_USERS:=40}"
: "${SIM_ROUNDS:=20}"
: "${SIM_INTERVAL:=750ms}"
: "${SIM_FUND_USDC:=100000}"

exec go run ./cmd/demo-sim \
  -rest-url "${GW_REST_URL}" \
  -ledger-addr "${LEDGER_ADDR}" \
  -matching-addr "${MATCHING_ADDR}" \
  -pg-dsn "${POSTGRES_DSN}" \
  -ticker "${DEMO_TICKER}" \
  -users "${SIM_USERS}" \
  -rounds "${SIM_ROUNDS}" \
  -interval "${SIM_INTERVAL}" \
  -fund-usdc "${SIM_FUND_USDC}" \
  "$@"
