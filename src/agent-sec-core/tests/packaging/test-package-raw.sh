#!/usr/bin/env bash
# Fixture-driven regression tests for the V2 raw package.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

BUILD="$TMP/build"
VERSION="$(python3 "$ROOT/packaging/raw/verify_release.py" \
    "$ROOT")"

install -d -m 0755 \
    "$BUILD/v2/bin" \
    "$BUILD/python-runtime/bin" \
    "$BUILD/python-runtime/lib/Tix8.4.3" \
    "$BUILD/python-runtime/lib/python3.11" \
    "$BUILD/python-runtime/lib/tk8.6/demos" \
    "$BUILD/openclaw-plugin" \
    "$BUILD/hermes-plugin/src" \
    "$BUILD/codex-plugin/hooks-plugin/.codex-plugin" \
    "$BUILD/codex-plugin/hooks-plugin/hooks" \
    "$BUILD/qoder-plugin/.qoder-plugin" \
    "$BUILD/qoder-plugin/hooks" \
    "$BUILD/qwen-code-extension/hooks" \
    "$BUILD/cosh-extension/hooks"

for binary in agent-sec-cli agent-sec-daemon agent-sec-daemon-health; do
    printf '#!/bin/sh\nexit 0\n' > "$BUILD/v2/bin/$binary"
    chmod 0755 "$BUILD/v2/bin/$binary"
done
printf '#!/bin/sh\nexit 0\n' > "$BUILD/linux-sandbox"
chmod 0755 "$BUILD/linux-sandbox"
printf '#!/bin/sh\nexit 0\n' > "$BUILD/python-runtime/bin/python3.11"
chmod 0755 "$BUILD/python-runtime/bin/python3.11"
printf 'license\n' > "$BUILD/python-runtime/lib/python3.11/LICENSE.txt"
printf 'license\n' > "$BUILD/python-runtime/lib/Tix8.4.3/license.terms"
printf 'license\n' > "$BUILD/python-runtime/lib/tk8.6/demos/license.terms"
cp "$ROOT/packaging/raw/assets/python-runtime/PROVENANCE.toml" \
    "$BUILD/python-runtime/PROVENANCE.toml"
(
    cd "$BUILD/python-runtime"
    python3 - <<'PY_DIGEST' > LICENSES.sha256
import hashlib
from pathlib import Path
for name in ["lib/Tix8.4.3/license.terms", "lib/python3.11/LICENSE.txt", "lib/tk8.6/demos/license.terms"]:
    print(hashlib.sha256(Path(name).read_bytes()).hexdigest() + "  " + name)
PY_DIGEST
)

cp "$ROOT/openclaw-plugin/openclaw.plugin.json" "$BUILD/openclaw-plugin/"
cp "$ROOT/hermes-plugin/src/plugin.yaml" "$BUILD/hermes-plugin/src/"
cp "$ROOT/codex-plugin/hooks-plugin/.codex-plugin/plugin.json" \
    "$BUILD/codex-plugin/hooks-plugin/.codex-plugin/"
cp "$ROOT/codex-plugin/hooks-plugin/hooks/hooks.json" \
    "$BUILD/codex-plugin/hooks-plugin/hooks/"
cp "$ROOT/qoder-plugin/.qoder-plugin/plugin.json" "$BUILD/qoder-plugin/.qoder-plugin/"
cp "$ROOT/qoder-plugin/hooks/hooks.json" "$BUILD/qoder-plugin/hooks/"
cp "$ROOT/qwen-code-extension/qwen-extension.json" "$BUILD/qwen-code-extension/"
cp "$ROOT/cosh-extension/cosh-extension.json" "$BUILD/cosh-extension/"
make -C "$ROOT" stage-skills BUILD_DIR="$BUILD"

run_package() {
    local output="$1"

    BUILD_DIR="$BUILD" \
    OUTPUT_DIR="$output" \
    TARGET_OS=linux \
    TARGET_ARCH=x86_64 \
    SOURCE_DATE_EPOCH=1783656696 \
        "$ROOT/packaging/raw/package.sh" package
}

OUT_ONE="$TMP/out-one"
OUT_TWO="$TMP/out-two"
run_package "$OUT_ONE"
run_package "$OUT_TWO"

ARTIFACT="sec-core-${VERSION}-linux-x86_64.tar.gz"
cmp "$OUT_ONE/$ARTIFACT" "$OUT_TWO/$ARTIFACT"

STAGE="$TMP/stage"
make -C "$ROOT" stage-raw \
    BUILD_DIR="$BUILD" \
    DESTDIR="$STAGE" \
    TARGET_OS=linux \
    TARGET_ARCH=x86_64

for binary in agent-sec-cli agent-sec-daemon agent-sec-daemon-health agent-sec-python linux-sandbox; do
    test "$(stat -c '%a' "$STAGE/bin/$binary" 2>/dev/null || stat -f '%Lp' "$STAGE/bin/$binary")" = "755"
done
test ! -e "$STAGE/lib/agent-sec-core/python3.11/site-packages"
test ! -e "$STAGE/.anolisa"
grep -Fq 'WantedBy=multi-user.target' \
    "$STAGE/share/agent-sec-core/templates/agent-sec-core.service.in"

tar -tzf "$OUT_ONE/$ARTIFACT" > "$TMP/tar-list.txt"
for expected in \
    "./bin/agent-sec-cli" \
    "./bin/agent-sec-daemon" \
    "./bin/agent-sec-daemon-health" \
    "./bin/agent-sec-python" \
    "./lib/agent-sec-core/python3.11/runtime/bin/python3.11"; do
    grep -Fxq "$expected" "$TMP/tar-list.txt"
done
if grep -Eq 'site-packages/agent_sec_cli|agent-sec-cli-wrapper|agent-sec-daemon-wrapper' \
    "$TMP/tar-list.txt"; then
    echo "ERROR: V2 raw archive contains V1 CLI/daemon payload" >&2
    exit 1
fi

# A detached component source tree has no ANOLISA checkout or metadata.
DETACHED="$TMP/detached"
mkdir -p "$DETACHED"
cp -R "$ROOT/packaging" "$DETACHED/packaging"
for input in v2/Cargo.toml openclaw-plugin/openclaw.plugin.json \
    openclaw-plugin/package.json hermes-plugin/src/plugin.yaml \
    codex-plugin/hooks-plugin/.codex-plugin/plugin.json \
    qoder-plugin/.qoder-plugin/plugin.json qwen-code-extension/qwen-extension.json \
    cosh-extension/cosh-extension.json LICENSE; do
    mkdir -p "$DETACHED/$(dirname "$input")"
    cp "$ROOT/$input" "$DETACHED/$input"
done
COMPONENT_SOURCE_DIR= RAW_CONTRACT= BUILD_DIR="$BUILD" \
    TARGET_OS=linux TARGET_ARCH=x86_64 DESTDIR="$TMP/detached-stage" \
    "$DETACHED/packaging/raw/package.sh" stage
test ! -e "$TMP/detached-stage/.anolisa"

# Integration metadata remains an explicit, validated distribution input.
python3 - "$TMP/integration.toml" "$VERSION" <<'CONTRACT'
from pathlib import Path
import sys

text = f'[component]\nname = "sec-core"\nversion = "{sys.argv[2]}"\n'
text += 'dependencies = [' + ', '.join('{name = "' + name + '"}' for name in
    ('bubblewrap', 'gnupg', 'jq', 'nodejs', 'systemd')) + ']\n'
for framework, suffix in (('openclaw', '/'), ('hermes', '/src/'),
                          ('codex', '/hooks-plugin/'), ('qoder', '/'),
                          ('qwencode', '/'), ('cosh', '/')):
    text += f'[[adapters]]\nframework = "{framework}"\n[adapters.backends.rpm]\n'
    text += 'resource_root = "{sharedir}/agent-sec-core/adapters/' + framework + suffix + '"\n'
Path(sys.argv[1]).write_text(text)
CONTRACT
INTEGRATION_CONTRACT="${PACKAGING_TEST_CONTRACT:-$TMP/integration.toml}"
RAW_CONTRACT="$INTEGRATION_CONTRACT" BUILD_DIR="$BUILD" \
    TARGET_OS=linux TARGET_ARCH=x86_64 DESTDIR="$TMP/integrated-stage" \
    "$DETACHED/packaging/raw/package.sh" stage
cmp "$INTEGRATION_CONTRACT" "$TMP/integrated-stage/.anolisa/component.toml"

# The launcher works in both the new and the historical private runtime directory.
for library in lib/agent-sec-core lib/anolisa/sec-core; do
    prefix="$TMP/launcher-${library##*/}"
    mkdir -p "$prefix/bin" "$prefix/$library/python3.11/runtime/bin"
    cp "$ROOT/packaging/raw/assets/bin/agent-sec-python" "$prefix/bin/"
    cat > "$prefix/$library/python3.11/runtime/bin/python3.11" <<'PYTHON'
#!/bin/sh
if [ "$1" = -c ]; then printf '3.11.6\n'; else printf '%s\n' "$@"; fi
PYTHON
    chmod 0755 "$prefix/$library/python3.11/runtime/bin/python3.11"
    "$prefix/bin/agent-sec-python" fixture > "$TMP/launcher-result"
    grep -Fxq fixture "$TMP/launcher-result"
done

# Component release versioning never follows a distribution-owned metadata link.
mkdir -p "$TMP/version-tools"
printf '#!/bin/sh\nexit 0\n' > "$TMP/version-tools/npm"
chmod 0755 "$TMP/version-tools/npm"
for metadata in absent local linked; do
    version_source="$TMP/version-$metadata"
    cp -R "$DETACHED" "$version_source"
    mkdir -p "$version_source/scripts"
    cp "$ROOT/scripts/bump-version.sh" "$version_source/scripts/"
    if [ "$metadata" != absent ]; then
        mkdir "$version_source/.anolisa"
        printf '[component]\nversion = "%s"\n' "$VERSION" > "$TMP/metadata-$metadata.toml"
        if [ "$metadata" = linked ]; then
            ln -s "$TMP/metadata-$metadata.toml" "$version_source/.anolisa/component.toml"
        else
            cp "$TMP/metadata-$metadata.toml" "$version_source/.anolisa/component.toml"
        fi
    fi
    PATH="$TMP/version-tools:$PATH" "$version_source/scripts/bump-version.sh" 9.9.9 >/dev/null
    grep -Fxq 'version = "9.9.9"' "$version_source/v2/Cargo.toml"
    if [ "$metadata" = local ]; then
        grep -Fxq 'version = "9.9.9"' "$version_source/.anolisa/component.toml"
    elif [ "$metadata" = linked ]; then
        grep -Fxq "version = \"$VERSION\"" "$TMP/metadata-linked.toml"
    fi
done

echo "OK: agent-sec-core V2 raw package tests passed"
