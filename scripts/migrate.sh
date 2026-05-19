#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${ROOT_DIR}/.env"

ACTION="${1:-up}"
STEPS="${2:-}"
MIGRATIONS_DIR="${ROOT_DIR}/db/migrations"

if ! command -v migrate >/dev/null 2>&1; then
  echo "[migrate] golang-migrate is not installed."
  echo "[migrate] Install with: brew install golang-migrate"
  exit 1
fi

if [[ -f "${ENV_FILE}" ]]; then
  # shellcheck disable=SC2046
  export $(grep -v '^#' "${ENV_FILE}" | xargs)
fi

: "${POSTGRES_USER:=sarvex}"
: "${POSTGRES_PASSWORD:=sarvex}"
: "${POSTGRES_HOST:=localhost}"
: "${POSTGRES_PORT:=5432}"
: "${POSTGRES_DB:=sarvex}"

DATABASE_URL="postgres://${POSTGRES_USER}:${POSTGRES_PASSWORD}@${POSTGRES_HOST}:${POSTGRES_PORT}/${POSTGRES_DB}?sslmode=disable"

if [[ -n "${STEPS}" ]]; then
  migrate -path "${MIGRATIONS_DIR}" -database "${DATABASE_URL}" "${ACTION}" "${STEPS}"
else
  migrate -path "${MIGRATIONS_DIR}" -database "${DATABASE_URL}" "${ACTION}"
fi
