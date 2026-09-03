#!/usr/bin/env bash

set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 COSH_ARCHIVE_ROOT" >&2
  exit 2
fi

ARCHIVE_ROOT="$1"
ACTION_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CONTRACT_SOURCE="src/aw/crates/aw-contracts"
VENDOR_ROOT="${ARCHIVE_ROOT}/vendor/aw-contracts"
SOURCE_PATH='path = "../../../aw/crates/aw-contracts"'
VENDORED_PATH='path = "../../vendor/aw-contracts"'

if [ ! -f "${ARCHIVE_ROOT}/Cargo.toml" ]; then
  echo "ERROR: COSH archive root is invalid: ${ARCHIVE_ROOT}" >&2
  exit 1
fi
if [ ! -f "${CONTRACT_SOURCE}/Cargo.toml" ]; then
  echo "ERROR: AW Contracts source is missing: ${CONTRACT_SOURCE}" >&2
  exit 1
fi

mkdir -p "${ARCHIVE_ROOT}/vendor"
cp -r "${CONTRACT_SOURCE}" "${VENDOR_ROOT}"

# Workspace-inherited package fields would bind the copied crate to the COSH
# workspace and change its locked package identity. The archive-only manifest
# keeps the released AW contract identity independent of the repository layout.
cp "${ACTION_ROOT}/aw-contracts-vendored.toml" "${VENDOR_ROOT}/Cargo.toml"

for manifest in \
  "${ARCHIVE_ROOT}/crates/cosh-gateway-contracts/Cargo.toml" \
  "${ARCHIVE_ROOT}/crates/cosh-core/Cargo.toml"; do
  if ! grep -Fq "${SOURCE_PATH}" "${manifest}"; then
    echo "ERROR: expected AW Contracts dependency is missing: ${manifest}" >&2
    exit 1
  fi
  sed -i.bak "s|${SOURCE_PATH}|${VENDORED_PATH}|" "${manifest}"
  rm -f "${manifest}.bak"
  if grep -Fq "${SOURCE_PATH}" "${manifest}" || ! grep -Fq "${VENDORED_PATH}" "${manifest}"; then
    echo "ERROR: failed to rewrite AW Contracts dependency: ${manifest}" >&2
    exit 1
  fi
done
