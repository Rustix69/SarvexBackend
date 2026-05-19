#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROTO_ROOT="${ROOT_DIR}/proto"
GO_OUT="${ROOT_DIR}/gen/go"
CPP_OUT="${ROOT_DIR}/gen/cpp"

if ! find "${PROTO_ROOT}" -type f -name "*.proto" -print -quit | grep -q .; then
  echo "[proto-gen] No .proto files found under ${PROTO_ROOT}; skipping generation."
  exit 0
fi

if ! command -v buf >/dev/null 2>&1; then
  echo "[proto-gen] buf is not installed. Install buf to generate bindings."
  exit 1
fi

mkdir -p "${GO_OUT}" "${CPP_OUT}"

echo "[proto-gen] Linting proto files..."
(
  cd "${ROOT_DIR}"
  buf lint
)

echo "[proto-gen] Generating code..."
(
  cd "${ROOT_DIR}"
  buf generate --template "${ROOT_DIR}/buf.gen.yaml"
)

echo "[proto-gen] Generation complete."
