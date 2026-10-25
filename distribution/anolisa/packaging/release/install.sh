#!/bin/sh
# Delegate local installation to the exact CLI shipped in this release.
set -eu
bundle=$(CDPATH='' cd -P -- "$(dirname -- "$0")" && pwd -P)
cli="$bundle/packages/anolisa-cli/payload/bin/anolisa"
"$cli" release install --bundle "$bundle" "$@"
for argument in "$@"; do
    case "$argument" in
        --dry-run|--json|--quiet|-q|-h|--help) exit 0 ;;
    esac
done
exec "$cli" setup
