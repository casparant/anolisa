# Installation and first run

[中文版](../../../zh/user-entrypoint/aw/installation.md)

AW requires Linux. The first-run example below also requires a signed-in Qoder
CLI 1.1.64 or later; package installation itself does not require an Agent.
For other Agents, see [Configuration and Agent launch](configuration.md).

## Get a package

AW uses Preview packages; ANOLISA CLI and RPM installation are not available yet.
Download the `aw-preview-linux-x86_64-…` artifact from a
[successful AW Integration run](https://github.com/agentic-os-org/ANOLISA/actions/workflows/aw-preview.yml).
Extract the ZIP to obtain the packages and `SHA256SUMS`. CI packages target Linux x86_64
with glibc 2.39 or later, such as Ubuntu 24.04.

To compile yourself or use Linux aarch64, follow [Build from source](../../../../developer-guide/en/aw/building.md)
and then continue here. Building requires network access to download dependencies.

| Package | Contents |
| --- | --- |
| `aw-core` | AW launch and configuration tools |
| `aw-provider-tokenless` | Tokenless and its required RTK; install core first |
| `aw-provider-sec-core` | Security Provider and sec-core programs; install core first |
| `aw-all-in-one` | All three components above |

These are format 1 Preview packages. Split packages must come from the same
build. Install all-in-one directly into a new prefix; do not also install the
split packages there. Their manifest and receipt format remain unchanged.

## Install

Run from the extracted artifact directory. Match the version to the downloaded filenames.
Use core for a first trial, or set `AW_BUNDLE_KIND` to `aw-all-in-one` for all capabilities.

```bash
sha256sum -c SHA256SUMS
export AW_VERSION=0.1.0-preview.1
export AW_BUNDLE_KIND=aw-core
export AW_BUNDLE="$AW_BUNDLE_KIND-$AW_VERSION-linux-$(uname -m)"
export AW_PREFIX="/opt/aw-preview/$AW_VERSION"
tar -xzf "$AW_BUNDLE.tar.gz"
sudo install -d -m 755 /opt/aw-preview
sudo "./$AW_BUNDLE/aw-package" install --prefix "$AW_PREFIX"
export PATH="$AW_PREFIX/bin:$PATH"
aw --help
```

Every checksum should report `OK`; a successful install prints `Installed verified AW components`.
Checksums establish consistency with the downloaded manifest, not publisher identity.
The installer refuses to overwrite existing components. [Use a new prefix when upgrading](operations.md).
Set `AW_PREFIX` and `PATH` in subsequent terminals too, or add them to your shell configuration.

For a personal installation that does not host a system backend, use an absolute `AW_PREFIX`
you own and omit `sudo` from the install command. Other users must not be able to write to its
parent directory. A root backend needs a root-managed installation.

## Preview a local Preview-package installation

If AW is already installed, the same local installer is available through:

```bash
aw install --prefix "$AW_PREFIX" --bundle "/absolute/path/to/unpacked-bundle" --dry-run
aw install --prefix "$AW_PREFIX" --bundle "/absolute/path/to/unpacked-bundle" --dry-run --json
aw install --prefix "$AW_PREFIX" --bundle "/absolute/path/to/unpacked-bundle"
```

The first two commands only read the bundle and target; they create no prefix,
lock or installation receipt. The Plan uses the same categories for core, split
Provider packages and all-in-one: `Source`, `Components`, `Files`, `Privileges`,
`Activation` and `Conflicts`. It checks payload checksums and reports known
dependency or overwrite conflicts. An empty conflict list is not a reservation:
lock availability, effective write access and disk capacity remain unchecked,
and installation rechecks inputs when it executes.

`--json` currently requires `--dry-run`. A conflict produces the Plan and exit
code `1`; invalid input may fail before a Plan can be produced. Without
`--dry-run`, a conflict-free Plan is followed by installation without an extra
confirmation prompt. AW uses the current user's permissions and never invokes
`sudo` automatically. Choose a new prefix for a component already installed there.

This format 1 workflow accepts unpacked Preview bundles. Its Provider packages
still require core from the same build. Component names, download URLs and
RPM/DEB packages are not supported.
It does not start services, register Agent Hooks or enable Providers.
The standalone `aw-package` installer remains available for bootstrap and compatibility.

## Install an open local bundle

Format 2 lets an author supply a local tool or Provider without joining a catalog
or using AW's version. This prototype supports only a new user-owned raw prefix:
run as a non-root user, with an existing parent directory you own that others
cannot write. Existing prefixes, including format 1 installations, cannot be
overwritten, adopted or extended. No sources are queried or dependencies downloaded.

The following Linux shell example creates `example.mant`, a harmless local test
script. It is not the manT project's software or release, and does not demonstrate
the real project's compatibility. Run with an already installed `aw` on `PATH`:

```bash
AW_DEMO_ROOT="$(mktemp -d "$HOME/aw-local-demo.XXXXXX")"
AW_DEMO_BUNDLE="$AW_DEMO_ROOT/bundle"
AW_DEMO_PREFIX="$AW_DEMO_ROOT/installed"
mkdir -p "$AW_DEMO_BUNDLE/payload/bin"
printf '%s\n' '#!/bin/sh' 'echo "format-2 fixture, not the manT project"' \
  > "$AW_DEMO_BUNDLE/payload/bin/mant-demo"
chmod 755 "$AW_DEMO_BUNDLE/payload/bin/mant-demo"
AW_DEMO_SIZE="$(wc -c < "$AW_DEMO_BUNDLE/payload/bin/mant-demo")"
AW_DEMO_SHA="$(sha256sum "$AW_DEMO_BUNDLE/payload/bin/mant-demo" | cut -d ' ' -f 1)"
AW_DEMO_ARCH="$(uname -m)"
cat > "$AW_DEMO_BUNDLE/manifest.json" <<EOF
{
  "format": 2,
  "packages": [{
    "id": "example.mant",
    "version": "0.1.0-demo",
    "kind": "tool",
    "platform": {"os": "linux", "arch": "$AW_DEMO_ARCH"},
    "requires": [],
    "files": {
      "bin/mant-demo": {"size": $AW_DEMO_SIZE, "sha256": "$AW_DEMO_SHA", "mode": 493}
    }
  }]
}
EOF
aw install --bundle "$AW_DEMO_BUNDLE" --prefix "$AW_DEMO_PREFIX" --dry-run
aw install --bundle "$AW_DEMO_BUNDLE" --prefix "$AW_DEMO_PREFIX" --allow-unverified
aw list --installed --prefix "$AW_DEMO_PREFIX" --json
aw doctor --prefix "$AW_DEMO_PREFIX"
```

The manifest sits beside `payload/`; file keys are paths relative to the installed
prefix. `mode` is a JSON decimal integer: `493` means `0755`, and `420` means
`0644`. Hashes are lowercase SHA-256; `size` is the exact byte count. Unknown
fields, links, special files and paths outside `bin/`, `libexec/` or `share/`
are rejected. Limits are 4 MiB of manifest, 64 packages, 8,192 files, 512 MiB
per file and 2 GiB of payloads in total.

IDs are namespaced lowercase ASCII names such as `example.mant`, not authenticated
publisher identities. Each package has its own version. `requires` contains exact
`{"id": "example.dependency", "version": "1.0.0"}` entries; all dependencies must
be included in the same bundle, without cycles. There are no version ranges or
dependencies on another prefix. For an actual Provider, use `"kind": "provider"`
and add `"provider": {"protocol": "aw-provider/v1alpha1", "executable": "bin/my-provider"}`;
that entrypoint must be a `0755` file owned by the same package. A `tool` must
not declare `provider`. Declaring a protocol does not test its implementation.

The same Plan categories apply. `--dry-run` needs no consent and writes nothing;
add `--json` for its structured form. Actual installation requires
`--allow-unverified` each time: checksums verify content consistency, not publisher
identity, and this option does not save a trusted source. AW copies files and
writes `.aw-installation.json`, recording `user` scope, the installing UID and
`raw` backend. It does not execute package scripts, change `PATH`, start services,
register Providers or connect Agent Hooks. An installed Provider still needs
[explicit configuration](configuration.md#edit-provider-declarations).

To remove this entire demo installation, keep the existing AW binary on `PATH`:

```bash
aw uninstall --prefix "$AW_DEMO_PREFIX"
```

Uninstall verifies recorded files and preserves unknown files. It does not check
running references; stop users of the prefix first. The source bundle under
`AW_DEMO_ROOT/bundle` is not removed. See [operations](operations.md) for details.

## Create configuration

Create configuration as your ordinary user, keeping data outside the installation prefix.

```bash
export AW_HOME="$HOME/aw"
export AW_QODER="$(command -v qodercli)"
install -d -m 700 "$AW_HOME" "$AW_HOME/workspace"
qodercli --version
aw config init --prefix "$AW_PREFIX" \
  --config "$AW_HOME/aw.yaml" --state-dir "$AW_HOME/state" \
  --qoder "$AW_QODER"
aw validate --config "$AW_HOME/aw.yaml"
```

`AW_QODER` must be the absolute path to the real executable. `config init` creates a complete file
and refuses to overwrite it. Enable [sec-core](security.md) or [Tokenless](tokenless.md) separately.

## Launch and verify

```bash
cd "$AW_HOME/workspace"
aw run --config "$AW_HOME/aw.yaml" --agent qoder
```

Expect AW instance information followed by the Qoder interface. Ask Qoder to read the current
directory with a tool, following its native permission prompts. After the tool returns, exit Qoder
and run these commands in the same terminal.

```bash
aw status --config "$AW_HOME/aw.yaml"
aw stop --config "$AW_HOME/aw.yaml"
```

After enabling a Provider, follow its guide to verify the effect.
