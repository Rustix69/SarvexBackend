#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

echo "[reset-demo] Recreating Milestone 0 infrastructure volumes..."
make reset
echo "[reset-demo] Reset complete."
