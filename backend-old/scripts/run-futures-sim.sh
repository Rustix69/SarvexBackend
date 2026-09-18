#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

: "${GW_REST_URL:=http://localhost:18080}"
: "${LEDGER_ADDR:=localhost:50062}"
: "${MATCHING_ADDR:=localhost:50064}"
: "${POSTGRES_DSN:=postgres://sarvaex:sarvaex@localhost:15432/sarvaex?sslmode=disable}"
: "${FUTURES_TICKERS:=SXF-FFUB-26OCT,SXF-FFUB-26DEC,SXF-USCPIYOY-26SEP,SXF-USCPIMOM-26SEP,SXF-AAAGAS-26OCT19,SXF-NFP-26SEP,SXF-SPX-26OCT16,SXF-NDX-26OCT16,SXF-SPXY-26DEC31}"
: "${SIM_USERS:=40}"
: "${SIM_ROUNDS:=20}"
: "${SIM_INTERVAL:=750ms}"
: "${SIM_FUND_USDC:=100000}"

exec go run ./cmd/demo-sim \
  -rest-url "${GW_REST_URL}" \
  -ledger-addr "${LEDGER_ADDR}" \
  -matching-addr "${MATCHING_ADDR}" \
  -pg-dsn "${POSTGRES_DSN}" \
  -ticker "${FUTURES_TICKERS}" \
  -users "${SIM_USERS}" \
  -rounds "${SIM_ROUNDS}" \
  -interval "${SIM_INTERVAL}" \
  -fund-usdc "${SIM_FUND_USDC}" \
  "$@"
