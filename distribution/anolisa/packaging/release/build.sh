#!/usr/bin/env bash
# Build a complete Linux release or assemble explicit prebuilt package inputs.
set -euo pipefail
SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
exec "${PYTHON:-python3}" "$SCRIPT_DIR/build.py" "$@"
