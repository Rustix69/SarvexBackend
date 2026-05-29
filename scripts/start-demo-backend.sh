#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

ENV_FILE="${ENV_FILE:-.env.example}"
POSTGRES_PORT="${POSTGRES_PORT:-15432}"
REDIS_PORT="${REDIS_PORT:-16379}"
GW_REST_HTTP_PORT="${GW_REST_HTTP_PORT:-18080}"
LEDGER_GRPC_PORT="${LEDGER_GRPC_PORT:-50062}"
ME_CORE_GRPC_PORT="${ME_CORE_GRPC_PORT:-50064}"
SIM_USERS="${SIM_USERS:-10}"
SIM_INTERVAL="${SIM_INTERVAL:-900ms}"
SIM_FUND_USDC="${SIM_FUND_USDC:-100000000}"
SIM_DIR="${SIM_DIR:-${ROOT_DIR}/.cache/demo-runtime}"
SIM_BIN="${SIM_DIR}/sarvex-demo-sim"

BINARY_TICKERS="${BINARY_TICKERS:-DEMO-AI-DEC26-1T,DEMO-BTC-JUN26-120K,DEMO-ETH-JUN26-8K,DEMO-FED-JUL26-CUT,DEMO-INDIA-GDP-Q2-26-7PCT,DEMO-NVIDIA-AUG26-5T,DEMO-OIL-MAY26-95,DEMO-TESLA-Q2-26-500K,DEMO-US-HOUSE-2026-DEM,DEMO-WC-2026-FRANCE,RBI-JUN26-CUT25}"
FUTURES_TICKERS="${FUTURES_TICKERS:-INDIA-CPI-JUN26-SCALAR,FUT-INDIA-GDP-FY26-SCALAR,FUT-BTC-JUN26-LEVEL,FUT-ETH-JUN26-LEVEL,FUT-AI-MCAP-DEC26-SCALAR,FUT-INDIA-UNEMP-DEC26-SCALAR,FUT-USDINR-DEC26-SCALAR,FUT-NIFTY-DEC26-LEVEL,FUT-FEDRATE-DEC26-SCALAR}"

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
    -pg-dsn "postgres://sarvaex:sarvaex@localhost:${POSTGRES_PORT}/sarvaex?sslmode=disable" \
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
