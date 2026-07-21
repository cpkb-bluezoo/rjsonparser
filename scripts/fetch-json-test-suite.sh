#!/usr/bin/env bash
# Clone https://github.com/nst/JSONTestSuite for local / CI-parity testing.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${ROOT}/JSONTestSuite"
REPO="https://github.com/nst/JSONTestSuite.git"

if [[ -d "${DEST}/test_parsing" ]]; then
  echo "JSONTestSuite already present at ${DEST}"
  exit 0
fi

if [[ -e "${DEST}" ]]; then
  echo "error: ${DEST} exists but has no test_parsing/; remove it and re-run" >&2
  exit 1
fi

git clone --depth 1 "${REPO}" "${DEST}"
echo "Cloned into ${DEST}"
