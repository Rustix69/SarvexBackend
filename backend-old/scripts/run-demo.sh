#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

echo "[run-demo] Starting Milestone 0 infrastructure..."
make run
echo "[run-demo] Done. Next: Milestone 1 (proto freeze) and Milestone 2 (migrations)."
