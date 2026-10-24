#!/bin/sh
# Delegate local installation to the exact CLI shipped in this release.
set -eu
bundle=$(CDPATH='' cd -P -- "$(dirname -- "$0")" && pwd -P)
exec "$bundle/packages/anolisa-cli/payload/bin/anolisa" \
    release install --bundle "$bundle" "$@"
