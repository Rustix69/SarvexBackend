#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROTO_ROOT="${ROOT_DIR}/proto"
GO_OUT="${ROOT_DIR}/gen/go"
CPP_OUT="${ROOT_DIR}/services/me-core/gen"
BUF_BIN="${BUF_BIN:-}"

if [[ -z "${BUF_BIN}" ]]; then
  if [[ -x "${HOME}/go/bin/buf" ]]; then
    BUF_BIN="${HOME}/go/bin/buf"
  else
    BUF_BIN="buf"
  fi
fi

if ! find "${PROTO_ROOT}" -type f -name "*.proto" -print -quit | grep -q .; then
  echo "[proto-gen] No .proto files found under ${PROTO_ROOT}; skipping generation."
  exit 0
fi

if ! command -v "${BUF_BIN}" >/dev/null 2>&1; then
  echo "[proto-gen] buf is not installed. Install buf to generate bindings."
  exit 1
fi

mkdir -p "${GO_OUT}" "${CPP_OUT}"

# Keep Buf cache in repo to avoid host-level cache permission issues.
export XDG_CACHE_HOME="${ROOT_DIR}/.cache"
export BUF_CACHE_DIR="${ROOT_DIR}/.cache/buf"
mkdir -p "${XDG_CACHE_HOME}" "${BUF_CACHE_DIR}"

echo "[proto-gen] Linting proto files..."
(
  cd "${ROOT_DIR}"
  "${BUF_BIN}" lint
)

echo "[proto-gen] Generating code..."
(
  cd "${ROOT_DIR}"
  "${BUF_BIN}" generate --template "${ROOT_DIR}/buf.gen.yaml"
)

echo "[proto-gen] Generation complete."
