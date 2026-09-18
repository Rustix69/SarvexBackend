#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${ENV_FILE:-${ROOT_DIR}/.env}"
RETENTION_HOURS="${DEMO_CLEANUP_RETENTION_HOURS:-24}"
APPLY=false

usage() {
  cat <<'EOF'
Usage:
  scripts/cleanup-demo-db.sh              Report cleanup candidates and table sizes
  scripts/cleanup-demo-db.sh --apply     Delete eligible demo trade data

Required for --apply:
  DEMO_CLEANUP_ENABLED=true

Optional:
  DEMO_CLEANUP_RETENTION_HOURS=24
EOF
}

case "${1:-}" in
  "") ;;
  --apply) APPLY=true ;;
  --help|-h) usage; exit 0 ;;
  *) usage >&2; exit 2 ;;
esac

if [[ -f "${ENV_FILE}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
  set +a
fi

if ! [[ "${RETENTION_HOURS}" =~ ^[1-9][0-9]*$ ]]; then
  echo "DEMO_CLEANUP_RETENTION_HOURS must be a positive integer" >&2
  exit 2
fi

if [[ "${APPLY}" == true && "${DEMO_CLEANUP_ENABLED:-false}" != "true" ]]; then
  echo "Refusing to apply cleanup: set DEMO_CLEANUP_ENABLED=true" >&2
  exit 1
fi

: "${POSTGRES_USER:=sarvex}"
: "${POSTGRES_PASSWORD:=sarvex}"
: "${POSTGRES_HOST:=localhost}"
: "${POSTGRES_PORT:=5432}"
: "${POSTGRES_DB:=sarvex}"

if ! command -v psql >/dev/null 2>&1; then
  echo "psql is required" >&2
  exit 1
fi

PSQL_BASE=(psql "postgresql://${POSTGRES_USER}:${POSTGRES_PASSWORD}@${POSTGRES_HOST}:${POSTGRES_PORT}/${POSTGRES_DB}?sslmode=disable" -v ON_ERROR_STOP=1)

if [[ "${APPLY}" != true ]]; then
  echo "[cleanup-demo-db] Report only; no rows will be deleted."
  "${PSQL_BASE[@]}" -v retention_hours="${RETENTION_HOURS}" <<'SQL'
SELECT current_database() AS database,
       now() - (:'retention_hours' || ' hours')::interval AS cutoff;

SELECT schemaname || '.' || relname AS table_name,
       pg_size_pretty(pg_total_relation_size(relid)) AS total_size,
       n_live_tup AS estimated_rows
FROM pg_stat_user_tables
WHERE schemaname IN ('orders', 'position', 'ledger', 'audit')
ORDER BY pg_total_relation_size(relid) DESC;

SELECT 'terminal simulator orders' AS candidate,
       count(*) AS rows
FROM orders.orders
WHERE user_id LIKE 'u_sim_%'
  AND status IN ('FILLED', 'CANCELLED', 'REJECTED', 'EXPIRED')
  AND updated_at < now() - (:'retention_hours' || ' hours')::interval
UNION ALL
SELECT 'posted simulator fills', count(*)
FROM orders.fills f
JOIN orders.orders maker ON maker.order_id = f.maker_order_id
JOIN orders.orders taker ON taker.order_id = f.taker_order_id
WHERE f.ledger_post_status = 'POSTED'
  AND f.ts < now() - (:'retention_hours' || ' hours')::interval
  AND maker.user_id LIKE 'u_sim_%'
  AND taker.user_id LIKE 'u_sim_%'
  AND maker.status IN ('FILLED', 'CANCELLED', 'REJECTED', 'EXPIRED')
  AND taker.status IN ('FILLED', 'CANCELLED', 'REJECTED', 'EXPIRED')
UNION ALL
SELECT 'closed simulator holds', count(*)
FROM ledger.holds
WHERE user_id LIKE 'u_sim_%'
  AND status = 'CLOSED'
  AND closed_at < now() - (:'retention_hours' || ' hours')::interval
  AND NOT EXISTS (
    SELECT 1 FROM orders.orders o
    WHERE o.hold_id = ledger.holds.hold_id
      AND o.status IN ('PENDING', 'OPEN', 'PARTIAL')
  )
UNION ALL
SELECT 'simulator audit events', count(*)
FROM audit.events
WHERE ts < now() - (:'retention_hours' || ' hours')::interval
  AND payload::text LIKE '%u_sim_%';
SQL
  exit 0
fi

echo "[cleanup-demo-db] Applying demo cleanup older than ${RETENTION_HOURS} hours..."
"${PSQL_BASE[@]}" -v retention_hours="${RETENTION_HOURS}" <<'SQL'
BEGIN;

-- Prevent overlapping cleanup runs. This lock does not alter exchange sequencing.
SELECT pg_advisory_xact_lock(hashtextextended('sarvex.demo.db.cleanup', 0));

CREATE TEMP TABLE cleanup_orders ON COMMIT DROP AS
SELECT order_id
FROM orders.orders
WHERE user_id LIKE 'u_sim_%'
  AND status IN ('FILLED', 'CANCELLED', 'REJECTED', 'EXPIRED')
  AND updated_at < now() - (:'retention_hours' || ' hours')::interval;

CREATE TEMP TABLE cleanup_fills ON COMMIT DROP AS
SELECT f.fill_id
FROM orders.fills f
JOIN orders.orders maker ON maker.order_id = f.maker_order_id
JOIN orders.orders taker ON taker.order_id = f.taker_order_id
JOIN cleanup_orders maker_candidate ON maker_candidate.order_id = f.maker_order_id
JOIN cleanup_orders taker_candidate ON taker_candidate.order_id = f.taker_order_id
WHERE f.ledger_post_status = 'POSTED'
  AND f.ts < now() - (:'retention_hours' || ' hours')::interval
  AND maker.user_id LIKE 'u_sim_%'
  AND taker.user_id LIKE 'u_sim_%';

CREATE TEMP TABLE cleanup_holds ON COMMIT DROP AS
SELECT hold_id
FROM ledger.holds
WHERE user_id LIKE 'u_sim_%'
  AND status = 'CLOSED'
  AND closed_at < now() - (:'retention_hours' || ' hours')::interval
  AND NOT EXISTS (
    SELECT 1 FROM orders.orders o
    WHERE o.hold_id = ledger.holds.hold_id
      AND o.status IN ('PENDING', 'OPEN', 'PARTIAL')
  );

CREATE TEMP TABLE cleanup_audit ON COMMIT DROP AS
SELECT event_id
FROM audit.events
WHERE ts < now() - (:'retention_hours' || ' hours')::interval
  AND payload::text LIKE '%u_sim_%';

SELECT 'fills' AS target, count(*) AS rows FROM cleanup_fills
UNION ALL
SELECT 'orders', count(*) FROM cleanup_orders
UNION ALL
SELECT 'closed holds', count(*) FROM cleanup_holds
UNION ALL
SELECT 'simulator audit events', count(*) FROM cleanup_audit;

-- Outbox rows are delivery state; the corresponding fills are already posted.
DELETE FROM orders.fill_posting_outbox
WHERE fill_id IN (SELECT fill_id FROM cleanup_fills);

-- These are demo trade facts only. Ledger postings remain immutable and are not removed.
DELETE FROM orders.fills
WHERE fill_id IN (SELECT fill_id FROM cleanup_fills);

DELETE FROM orders.orders o
WHERE o.order_id IN (SELECT order_id FROM cleanup_orders)
  AND NOT EXISTS (
    SELECT 1 FROM orders.fills f
    WHERE f.maker_order_id = o.order_id OR f.taker_order_id = o.order_id
  );

DELETE FROM ledger.hold_operations
WHERE hold_id IN (SELECT hold_id FROM cleanup_holds);

DELETE FROM ledger.holds
WHERE hold_id IN (SELECT hold_id FROM cleanup_holds);

DELETE FROM audit.events
WHERE event_id IN (SELECT event_id FROM cleanup_audit);

COMMIT;
SQL

# VACUUM reclaims deleted pages for reuse without blocking the live demo.
"${PSQL_BASE[@]}" -c "VACUUM (ANALYZE) orders.orders, orders.fills, orders.fill_posting_outbox, ledger.holds, ledger.hold_operations, audit.events;"

echo "[cleanup-demo-db] Cleanup complete. Ledger, positions, refdata, oracle, and settlement history were preserved."
