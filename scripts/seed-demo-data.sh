#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${ROOT_DIR}/.env"

if [[ -f "${ENV_FILE}" ]]; then
  # shellcheck disable=SC2046
  export $(grep -v '^#' "${ENV_FILE}" | xargs)
fi

: "${POSTGRES_USER:=sarvex}"
: "${POSTGRES_PASSWORD:=sarvex}"
: "${POSTGRES_HOST:=localhost}"
: "${POSTGRES_PORT:=5432}"
: "${POSTGRES_DB:=sarvex}"

PSQL_BASE=(psql "postgresql://${POSTGRES_USER}:${POSTGRES_PASSWORD}@${POSTGRES_HOST}:${POSTGRES_PORT}/${POSTGRES_DB}?sslmode=disable" -v ON_ERROR_STOP=1)

echo "[seed-demo-data] Seeding demo users..."
"${PSQL_BASE[@]}" -f "${ROOT_DIR}/db/seed/demo_users.sql"

echo "[seed-demo-data] Seeding contracts/events..."
"${PSQL_BASE[@]}" -f "${ROOT_DIR}/db/seed/contracts.sql"

echo "[seed-demo-data] Seeding house accounts..."
"${PSQL_BASE[@]}" -f "${ROOT_DIR}/db/seed/house_accounts.sql"

echo "[seed-demo-data] Seed completed."
