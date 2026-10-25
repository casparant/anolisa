#!/usr/bin/env bash
# Stage and package agent-sec-core using its component-owned raw contract.
set -euo pipefail

die() {
    printf 'ERROR: %s\n' "$*" >&2
    exit 1
}

copy_tree() {
    local source="$1"
    local destination="$2"

    [ -d "$source" ] || die "missing build output $source"
    install -d -m 0755 "$destination"
    cp -a "$source"/. "$destination"/
}

copy_tree_dereferenced() {
    local source="$1"
    local destination="$2"

    [ -d "$source" ] || die "missing build output $source"
    install -d -m 0755 "$destination"
    cp -RLp "$source"/. "$destination"/
}

validate_bundled_python() {
    local runtime="$1"
    local required

    for required in \
        bin/python3.11 \
        lib/python3.11/LICENSE.txt \
        lib/Tix8.4.3/license.terms \
        lib/tk8.6/demos/license.terms \
        PROVENANCE.toml \
        LICENSES.sha256; do
        [ -f "$runtime/$required" ] || \
            die "bundled Python is missing $required"
    done
    [ -x "$runtime/bin/python3.11" ] || \
        die "bundled Python interpreter is not executable"
    cmp -s "$runtime/PROVENANCE.toml" \
        "$ROOT/packaging/raw/assets/python-runtime/PROVENANCE.toml" || \
        die "bundled Python provenance does not match the pinned runtime"
    python3 - "$runtime" <<'PY_CHECK' || die "bundled Python license manifest is invalid"
import hashlib
from pathlib import Path
import sys
runtime = Path(sys.argv[1]).resolve()
entries = (runtime / "LICENSES.sha256").read_text().splitlines()
if not entries:
    raise SystemExit("empty bundled Python license manifest")
for line in entries:
    expected, separator, relative = line.partition("  ")
    path = runtime / relative
    if (not separator or len(expected) != 64 or not path.resolve().is_relative_to(runtime)
            or not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != expected):
        raise SystemExit(f"invalid bundled Python license digest: {relative}")
PY_CHECK
}

normalize_modes() {
    local stage="$1"

    find "$stage" -type d -exec chmod 0755 {} +
    find "$stage" -type f -exec chmod 0644 {} +
    find "$stage/bin" -type f -exec chmod 0755 {} +
    find "$stage/lib/agent-sec-core/python3.11/runtime/bin" \
        -type f -exec chmod 0755 {} +
    find "$stage/share/agent-sec-core/adapters" "$stage/share/agent-sec-core/skills" \
        -type f \( -name '*.sh' -o -name '*.py' \) -exec chmod 0755 {} +
}

stage_payload() {
    local stage="$1"

    if [ -e "$stage" ] && [ -n "$(find "$stage" -mindepth 1 -print -quit)" ]; then
        die "DESTDIR must be empty: $stage"
    fi

    install -d -m 0755 \
        \
        "$stage/bin" \
        "$stage/lib/agent-sec-core/python3.11/runtime" \
        "$stage/share/agent-sec-core/adapters/openclaw" \
        "$stage/share/agent-sec-core/adapters/hermes" \
        "$stage/share/agent-sec-core/adapters/codex" \
        "$stage/share/agent-sec-core/adapters/qoder" \
        "$stage/share/agent-sec-core/adapters/qwencode" \
        "$stage/share/agent-sec-core/adapters/cosh" \
        "$stage/share/agent-sec-core/skills" \
        "$stage/share/agent-sec-core/templates" \
        "$stage/share/doc/sec-core"

    if [ -n "$CONTRACT" ]; then
        install -d -m 0755 "$stage/.anolisa"
        install -m 0644 "$CONTRACT" "$stage/.anolisa/component.toml"
    fi
    install -p -m 0755 "$BUILD_DIR/linux-sandbox" "$stage/bin/linux-sandbox"
    install -p -m 0755 "$BUILD_DIR/v2/bin/agent-sec-cli" "$stage/bin/agent-sec-cli"
    install -p -m 0755 "$BUILD_DIR/v2/bin/agent-sec-daemon" "$stage/bin/agent-sec-daemon"
    install -p -m 0755 "$BUILD_DIR/v2/bin/agent-sec-daemon-health" "$stage/bin/agent-sec-daemon-health"
    install -p -m 0755 "$ROOT/packaging/raw/assets/bin/agent-sec-python" \
        "$stage/bin/agent-sec-python"
    install -p -m 0644 \
        "$ROOT/packaging/systemd/agent-sec-core-v2.service.in" \
        "$stage/share/agent-sec-core/templates/agent-sec-core.service.in"
    install -p -m 0644 "$ROOT/LICENSE" "$stage/share/doc/sec-core/LICENSE"

    validate_bundled_python "$BUILD_DIR/python-runtime"
    copy_tree_dereferenced "$BUILD_DIR/python-runtime" \
        "$stage/lib/agent-sec-core/python3.11/runtime"
    copy_tree "$BUILD_DIR/openclaw-plugin" "$stage/share/agent-sec-core/adapters/openclaw"
    copy_tree "$BUILD_DIR/hermes-plugin/src" "$stage/share/agent-sec-core/adapters/hermes"
    if [ -d "$BUILD_DIR/hermes-plugin/scripts" ]; then
        copy_tree "$BUILD_DIR/hermes-plugin/scripts" \
            "$stage/share/agent-sec-core/adapters/hermes/scripts"
    fi
    copy_tree "$BUILD_DIR/codex-plugin/hooks-plugin" "$stage/share/agent-sec-core/adapters/codex"
    copy_tree "$BUILD_DIR/qoder-plugin" "$stage/share/agent-sec-core/adapters/qoder"
    copy_tree "$BUILD_DIR/qwen-code-extension" "$stage/share/agent-sec-core/adapters/qwencode"
    copy_tree "$BUILD_DIR/cosh-extension" "$stage/share/agent-sec-core/adapters/cosh"
    copy_tree "$BUILD_DIR/skills" "$stage/share/agent-sec-core/skills"

    python3 "$ROOT/packaging/raw/adapt_payload.py" "$stage"

    validate_bundled_python \
        "$stage/lib/agent-sec-core/python3.11/runtime"
    if [ -n "$(find "$stage/lib/agent-sec-core/python3.11/runtime" \
        -type l -print -quit)" ]; then
        die "bundled Python runtime contains a symbolic link"
    fi
    normalize_modes "$stage"
    python3 "$ROOT/packaging/raw/verify_release.py" \
        "$ROOT" "$CONTRACT" --payload-root "$stage" > /dev/null
}

resolve_epoch() {
    if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
        printf '%s\n' "$SOURCE_DATE_EPOCH"
        return
    fi
    git -C "$ROOT" log -1 --format=%ct -- . 2>/dev/null || \
        die "SOURCE_DATE_EPOCH is unset and the source commit time is unavailable"
}

COMMAND="${1:-}"
[ "$COMMAND" = "stage" ] || [ "$COMMAND" = "package" ] || \
    die "usage: $0 {stage|package}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="${COMPONENT_SOURCE_DIR:-$(cd "$SCRIPT_DIR/../.." && pwd -P)}"
BUILD_DIR="${BUILD_DIR:-$ROOT/target}"
CONTRACT="${RAW_CONTRACT:-}"
TARGET_OS="${TARGET_OS:-linux}"
TARGET_ARCH="${TARGET_ARCH:-$(uname -m)}"

[ "$TARGET_OS" = "linux" ] || die "raw packages support Linux only"
[ "$TARGET_ARCH" = "x86_64" ] || die "raw packages currently support x86_64 only"
[ -z "$CONTRACT" ] || [ -f "$CONTRACT" ] || die "raw contract not found: $CONTRACT"
[ -x "$BUILD_DIR/v2/bin/agent-sec-cli" ] || \
    die "missing V2 build output $BUILD_DIR/v2/bin/agent-sec-cli"
[ -x "$BUILD_DIR/v2/bin/agent-sec-daemon" ] || \
    die "missing V2 build output $BUILD_DIR/v2/bin/agent-sec-daemon"
[ -x "$BUILD_DIR/v2/bin/agent-sec-daemon-health" ] || \
    die "missing V2 build output $BUILD_DIR/v2/bin/agent-sec-daemon-health"
[ -x "$BUILD_DIR/linux-sandbox" ] || die "missing build output $BUILD_DIR/linux-sandbox"

VERSION="$(python3 "$ROOT/packaging/raw/verify_release.py" "$ROOT" "$CONTRACT")"

if [ "$COMMAND" = "stage" ]; then
    [ -n "${DESTDIR:-}" ] || die "DESTDIR is required by stage-raw"
    stage_payload "$DESTDIR"
    printf 'Staged sec-core %s raw payload at %s\n' "$VERSION" "$DESTDIR"
    exit 0
fi

ARCHIVE_TOOL="$SCRIPT_DIR/archive.py"
OUTPUT_DIR="${OUTPUT_DIR:-$BUILD_DIR/raw}"
EPOCH="$(resolve_epoch)"
case "$EPOCH" in
    ''|*[!0-9]*) die "SOURCE_DATE_EPOCH must be a non-negative integer" ;;
esac

WORK="$(mktemp -d)"
TEMP_ARTIFACT=""
cleanup() {
    rm -rf "$WORK"
    if [ -n "$TEMP_ARTIFACT" ]; then
        rm -f "$TEMP_ARTIFACT"
    fi
}
trap cleanup EXIT
STAGE="$WORK/stage"
stage_payload "$STAGE"

install -d -m 0755 "$OUTPUT_DIR"
ARTIFACT="sec-core-${VERSION}-${TARGET_OS}-${TARGET_ARCH}.tar.gz"
TEMP_ARTIFACT="$OUTPUT_DIR/.${ARTIFACT}.tmp.$$"
python3 "$ARCHIVE_TOOL" --source "$STAGE" --output "$TEMP_ARTIFACT" --epoch "$EPOCH"
mv -f "$TEMP_ARTIFACT" "$OUTPUT_DIR/$ARTIFACT"
TEMP_ARTIFACT=""
printf '%s\n' "$OUTPUT_DIR/$ARTIFACT"
