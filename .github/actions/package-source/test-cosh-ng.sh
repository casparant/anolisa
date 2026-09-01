#!/usr/bin/env bash

set -euo pipefail

REPOSITORY_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
TEMPORARY_ROOT="$(mktemp -d)"
trap 'rm -rf "${TEMPORARY_ROOT}"' EXIT

ARCHIVE_ROOT="${TEMPORARY_ROOT}/cosh-ng-source"
cp -r "${REPOSITORY_ROOT}/src/cosh-ng" "${ARCHIVE_ROOT}"

cd "${REPOSITORY_ROOT}"
"${REPOSITORY_ROOT}/.github/actions/package-source/vendor-cosh-aw-contracts.sh" \
  "${ARCHIVE_ROOT}"

for manifest in \
  "${ARCHIVE_ROOT}/crates/cosh-gateway-contracts/Cargo.toml" \
  "${ARCHIVE_ROOT}/crates/cosh-core/Cargo.toml"; do
  grep -Fq 'path = "../../vendor/aw-contracts"' "${manifest}"
  if grep -Fq 'path = "../../../aw/crates/aw-contracts"' "${manifest}"; then
    echo "ERROR: source archive retains a repository-relative dependency: ${manifest}" >&2
    exit 1
  fi
done

CARGO_TARGET_DIR="${REPOSITORY_ROOT}/src/cosh-ng/target/package-source-test" \
  cargo check \
    --manifest-path "${ARCHIVE_ROOT}/Cargo.toml" \
    --workspace \
    --locked

echo "COSH source archive dependency test passed"
