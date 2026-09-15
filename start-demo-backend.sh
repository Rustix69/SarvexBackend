#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

ENV_FILE="${ENV_FILE:-.env.example}"
if [[ -f "${ENV_FILE}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
  set +a
fi

POSTGRES_PORT="${POSTGRES_PORT:-15432}"
POSTGRES_USER="${POSTGRES_USER:-sarvaex}"
POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-sarvaex}"
POSTGRES_DB="${POSTGRES_DB:-sarvaex}"
REDIS_PORT="${REDIS_PORT:-16379}"
GW_REST_HTTP_PORT="${GW_REST_HTTP_PORT:-18080}"
LEDGER_GRPC_PORT="${LEDGER_GRPC_PORT:-50062}"
ME_CORE_GRPC_PORT="${ME_CORE_GRPC_PORT:-50064}"
SIM_USERS="${SIM_USERS:-10}"
SIM_INTERVAL="${SIM_INTERVAL:-900ms}"
SIM_FUND_USDC="${SIM_FUND_USDC:-10000000000}"
SIM_DIR="${SIM_DIR:-${ROOT_DIR}/.cache/demo-runtime}"
SIM_BIN="${SIM_DIR}/sarvex-demo-sim"

BINARY_TICKERS="${BINARY_TICKERS:-SX-FEDDEC-26OCT-H25,SX-FEDUB-26DEC-T4.00,SX-USCPIYOY-26SEP-T3.4,SX-USCPIMOM-26SEP-T0.2,SX-AAAGAS-26OCT19-T4.25,SX-NFP-26SEP-T100,SX-SPX-26OCT16-T7650,SX-NDX-26OCT16-T29250,SX-SPXY-26DEC31-B6700,SX-UST10Y-26OCT30-T4.90,SX-EURUSD-26OCT16-T1.1600,SX-BTCD-26OCT30H17-T80000,SX-BTC15M-{window},SX-BTCLOW-26DEC31-T60000,SX-ETHD-26OCT30H17-T2500,SX-WTI-26OCT30-T100,SX-WTIHI-26OCT-T110,SX-XAU-26OCT16-UP,SX-HIGHNY-26OCT15-B64.5,SX-RAINPHL-26OCT15,SX-HORMUZW-26OCT18-T40,SX-HORMUZN-27JAN01,SX-USHOUSE-26-R,SX-USSENATE-26-R,SX-PRESNOMD-28-{CAND},SX-NFL-{GAME}-{TEAM},SX-EPL-{MATCH}-{1X2},SX-ATP-{MATCH}-{PLAYER},SX-RBIDEC-26OCT-H25,SX-INCPI-26SEP-T4.50,SX-USDINR-26OCT30-T95.00,SX-NIFTY-26OCT27-T23500,SX-MONSOON-26-T90,SX-DELAQI-26NOV09-T400,SX-CRIC-INDWI-ODI1-IND,SX-AG26-IND-T100,SX-LBMAGOLD-26NOV06-T4500,SX-UP27-BJP-T202,SX-BRENT-26OCT30-T100,SX-CBUAE-26OCT-HIKE,SX-OPECP-26DEC-INC,SX-HIGHDXB-26OCT15-T38,SX-TASI-26OCT29-ATM,SX-USIRAN-CF-26OCT31,SX-SPL-RIYDERBY-HIL,SX-BOJDEC-26SEP-H25,SX-USDJPY-26OCT30-T155,SX-N225-26OCT30-ATM,SX-LPR1Y-26OCT-CUT,SX-HIGHTYO-26OCT01-B24,SX-BTCASIA-26OCT30H0800Z-T80000,SX-ECBDEC-26OCT-H25,SX-EZHICP-26OCT-T3.0,SX-TTF-26OCT30-ATM,SX-FRPRES27-LEPEN}"
FUTURES_TICKERS="${FUTURES_TICKERS:-SXF-FFUB-26OCT,SXF-FFUB-26DEC,SXF-USCPIYOY-26SEP,SXF-USCPIMOM-26SEP,SXF-AAAGAS-26OCT19,SXF-NFP-26SEP,SXF-SPX-26OCT16,SXF-NDX-26OCT16,SXF-SPXY-26DEC31,SXF-UST10Y-26OCT30,SXF-EURUSD-26OCT16,SXF-BTCD-26OCT30H17,SXF-BTCH-{hour},SXF-BTCLOW-26DEC31,SXF-ETHD-26OCT30H17,SXF-WTI-26OCT30,SXF-WTIHI-26OCT,SXF-XAU-26OCT16,SXF-HIGHNY-26OCT15,SXF-RAINPHL-26OCT15,SXF-HORMUZW-26OCT18,SXF-HORMUZMA-26DEC31,SXF-USHOUSE-R-26,SXF-USSENATE-R-26,SXF-NFLMGN-{GAME},SXF-EPLGD-{MATCH},SXF-ATPGAMES-{MATCH},SXF-RBIREPO-26OCT,SXF-INCPI-26SEP,SXF-USDINR-26OCT30,SXF-NIFTY-26OCT27,SXF-MONSOON-26,SXF-DELAQI-26NOV09,SXF-CRIC-INDWI-ODI1-RUNS,SXF-AG26-IND,SXF-LBMAGOLD-26NOV06,SXF-UP27-BJP,SXF-BRENT-26OCT30,SXF-CBUAE-26OCT,SXF-OPECP-26DEC,SXF-HIGHDXB-26OCT15,SXF-TASI-26OCT29,SXF-BRENT-26OCT30 (proxy),SXF-SPL-RIYDERBY-GD,SXF-BOJRATE-26SEP,SXF-USDJPY-26OCT30,SXF-N225-26OCT30,SXF-LPR1Y-26OCT,SXF-HIGHTYO-26OCT01,SXF-BTCASIA-26OCT30H0800Z,SXF-ECBDFR-26OCT,SXF-EZHICP-26OCT,SXF-TTF-26OCT30,SXF-FRPRES27-RN}"

export POSTGRES_PORT REDIS_PORT GW_REST_HTTP_PORT LEDGER_GRPC_PORT ME_CORE_GRPC_PORT

mkdir -p "${SIM_DIR}"

log() {
  printf '[start-demo-backend] %s\n' "$*"
}

stop_from_pidfile() {
  local pidfile="$1"
  if [[ ! -f "${pidfile}" ]]; then
    return
  fi
  local pid
  pid="$(cat "${pidfile}" 2>/dev/null || true)"
  if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
    kill "${pid}" 2>/dev/null || true
    sleep 1
    if kill -0 "${pid}" 2>/dev/null; then
      kill -9 "${pid}" 2>/dev/null || true
    fi
  fi
  rm -f "${pidfile}"
}

stop_legacy_simulators() {
  stop_from_pidfile "${SIM_DIR}/binary-sim.pid"
  stop_from_pidfile "${SIM_DIR}/futures-sim.pid"

  local pids
  pids="$(pgrep -f '(/tmp/sarvex-demo-sim|/sarvex-demo-sim|go run ./cmd/demo-sim|/demo-sim).* -continuous' || true)"
  if [[ -z "${pids}" ]]; then
    return
  fi
  log "Stopping old simulator processes: ${pids//$'\n'/ }"
  kill ${pids} 2>/dev/null || true
  sleep 1
  pids="$(pgrep -f '(/tmp/sarvex-demo-sim|/sarvex-demo-sim|go run ./cmd/demo-sim|/demo-sim).* -continuous' || true)"
  if [[ -n "${pids}" ]]; then
    kill -9 ${pids} 2>/dev/null || true
  fi
}

wait_for_http() {
  local url="$1"
  local name="$2"
  local tries="${3:-60}"
  for _ in $(seq 1 "${tries}"); do
    if curl -fsS "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  log "${name} did not become ready at ${url}"
  return 1
}

start_simulator() {
  local name="$1"
  local tickers="$2"
  local log_file="${SIM_DIR}/${name}.log"
  local pid_file="${SIM_DIR}/${name}.pid"

  stop_from_pidfile "${pid_file}"
  : > "${log_file}"

  setsid -f "${SIM_BIN}" \
    -rest-url "http://localhost:${GW_REST_HTTP_PORT}" \
    -ledger-addr "localhost:${LEDGER_GRPC_PORT}" \
    -matching-addr "localhost:${ME_CORE_GRPC_PORT}" \
    -pg-dsn "postgres://${POSTGRES_USER}:${POSTGRES_PASSWORD}@localhost:${POSTGRES_PORT}/${POSTGRES_DB}?sslmode=disable" \
    -ticker "${tickers}" \
    -users "${SIM_USERS}" \
    -interval "${SIM_INTERVAL}" \
    -fund-usdc "${SIM_FUND_USDC}" \
    -continuous \
    -reset-book \
    > "${log_file}" 2>&1 < /dev/null

  sleep 1
  pgrep -f "${SIM_BIN}.*${tickers%%,*}" | head -n 1 > "${pid_file}" || true
  if [[ ! -s "${pid_file}" ]]; then
    log "${name} simulator did not start. Log:"
    tail -n 80 "${log_file}" || true
    return 1
  fi
  log "${name} simulator started pid=$(cat "${pid_file}") log=${log_file}"
}

log "Starting Docker backend services..."
docker compose --env-file "${ENV_FILE}" up -d --build

log "Waiting for REST gateway..."
wait_for_http "http://localhost:${GW_REST_HTTP_PORT}/readyz" "gw-rest"

log "Building demo simulator binary..."
go build -o "${SIM_BIN}" ./cmd/demo-sim

log "Starting simulators..."
stop_legacy_simulators
start_simulator "binary-sim" "${BINARY_TICKERS}"
start_simulator "futures-sim" "${FUTURES_TICKERS}"

log "Waiting for live simulator health..."
for _ in $(seq 1 30); do
  summary="$(curl -fsS "http://localhost:${GW_REST_HTTP_PORT}/v1/health/overview" | jq -r '.summary | "\(.running)/\(.total) running, \(.not_running) down"' 2>/dev/null || true)"
  if [[ "${summary}" == *"0 down" ]]; then
    log "Health OK: ${summary}"
    log "Backend ready. Run frontend separately with: cd frontend && npm run dev"
    exit 0
  fi
  sleep 2
done

log "Backend started, but health is not fully green yet:"
curl -fsS "http://localhost:${GW_REST_HTTP_PORT}/v1/health/overview" | jq '{summary, down:[.items[] | select(.status!="running")]}' || true
exit 1
