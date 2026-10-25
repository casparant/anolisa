# ANOLISA distribution

[中文版](README_zh.md)

ANOLISA distribution tooling assembles independently delivered components into
complete product releases. Optional product integration lives in each source
root's `.anolisa/` directory; component code stays in `src/`, and Provider
implementations stay in `providers/`. Capability components own their standalone
RPM and tarball delivery. This directory provides shared packaging tools,
product composition, service policy and legacy installation migration.

ANOLISA calls `crates/local-package` directly for product raw installation;
AW CLI is not part of that call path. The shared library owns file verification,
package records, transactions and recovery. AW retains its runtime, configuration,
Agent Adapters, Provider protocol and local-extension compatibility interface.

## Maintainer entry points

| Task | Entry point |
|------|-------------|
| Change local file verification or transaction rules | `crates/local-package/` |
| Build historical AW Preview bundles | `tooling/aw-preview/` |
| Change standalone RPM/tarball delivery | The component's own Make targets, spec and packaging directory |
| Change ANOLISA adaptation of a component | `src/<component>/.anolisa/recipe.json` and optional `component.toml.in` |
| Change ANOLISA adaptation of a Provider | `providers/<provider>/.anolisa/`; implementation and independent delivery stay with the Provider |
| Change ANOLISA CLI's own product packaging | `distribution/anolisa/.anolisa/` |
| Change package combinations | `release/linux.json` |
| Change service presets or container allowlist | `release/services.json` |
| Stage already built component bytes | `packaging/component.py` |
| Produce a complete tarball | `anolisa/packaging/release/build.sh` |
| Prepare an optional product source snapshot | `packaging/snapshot.py`; component source builds keep their own entrypoints |
| Build a container from a verified bundle | `containers/Containerfile` |

`scripts/build-all.sh` remains the developer build/install convenience command.
Standalone builds retain real component-owned specs and packaging entrypoints.
Product recipes consume their delivered bytes. A component release does not
need ANOLISA CLI, product profiles or AW.

Components can continue sharing repository metadata such as the root `LICENSE`,
including existing source symlinks. Independent delivery does not require
duplicating the license in every component's source tree. Published archives
and packages include the actual license content when assembling their artifacts.

The source-local `.anolisa/` directory groups optional ANOLISA package mappings,
installation descriptions and product-specific packaging inputs. Independent
component owners can ignore or omit it and continue using their own RPM/raw
release entrypoints. Existing component specs, tarball scripts, service templates
and native integrations remain outside it when they also serve independent
delivery. `distribution/` discovers these optional integrations and owns the
shared product tools; it does not maintain another component/Provider recipe
tree. Source locations do not change package IDs or release tags.

This independence also applies to os-skills and ktuner. COSH has the product's
Agent role and keeps its distribution composition; this change does not add an
AW Adapter for COSH. AW remains independently usable, and Provider source paths
are repository locations rather than a runtime registration mechanism.

## Independent component delivery

CLI and daemon stay together in a backend package. Native Agent plugins and
skills are independently selectable. Old monolithic RPM names remain dependency
wrappers so a normal RPM upgrade retains the previously supplied integrations.
For example, `tokenless` pulls its backend and native plugins; an AW composition
selects `tokenless-core` and `aw-provider-tokenless` without every native plugin.
RPM retains native ownership; the product installer never imports those files
into a raw package database.

Use the existing component Make targets and raw packaging entry points to
produce independent artifacts. Independent source archives retain the component's own build and packaging
inputs and do not require a product checkout. `packaging/snapshot.py` supports
optional distribution snapshots; it is not the component's mandatory publisher.

To split an already published raw artifact without compiling it:

```bash
python3 distribution/packaging/component.py \
  --component tokenless --raw-input /absolute/tokenless.tar.gz \
  --version 0.8.8 --arch x86_64 --source-commit FULL_40_HEX_COMMIT \
  --output /absolute/new-package-directory
```

Replace the example version and commit with the artifact's verified provenance.
The output contains local package directories for the shared installation library.
An individual Provider may require `--dependency-version ID=VERSION` for
dependencies delivered separately. `--binary` stages a prebuilt Cargo component
instead of `--raw-input`.

Component resources use `share/<component>/adapters/<agent>/`,
`share/<component>/skills/` and `share/<component>/templates/`.
`share/anolisa/` contains product registration and composition metadata.
RPMs retain required old resource paths as same-version compatibility copies
owned by the corresponding subpackage. Configuration and data keep their
existing component locations; resource reorganization does not move databases.

Existing compatibility paths and identifiers can still contain `anolisa`.
A component-owned module may retain a historical name when the component ships
and maintains it independently. This naming exception does not permit a required
ANOLISA SDK, CLI or product service dependency in the component's regular CLI,
daemon or native Agent plugins. Remove such product coupling or put it in an
optional ANOLISA integration package. Keep legacy resource and plugin identities
compatible without treating every historical name as a runtime dependency.

## Complete product releases

On a supported Linux build host, build a pinned source release:

```bash
distribution/anolisa/packaging/release/build.sh --output-dir "$PWD/target/product-release"
```

To assemble an already prepared complete package inventory, use
`--staged-input DIR --source-commit COMMIT --epoch UNIX_SECONDS`; this path
does not compile. The archive includes all distributable packages and a
schema-2 `release.json`. Installation selects only the requested profile and
its dependencies. Every delivered payload is verified before selection.

| Profile | Scope | Contents |
|---------|-------|----------|
| `complete` | user or system | Every delivered package, including backends, native integrations, Providers and skills |
| `minimal` | user or system | AW and ANOLISA CLI; explicit reduced selection |
| `standard-user` | user | Supported user components, skills and Tokenless Provider |
| `standard-system` | system | Backend components, skills and implemented Providers |
| `native-<agent>` | system | Standard backends plus available native integration packages for that Agent |

The default is `complete` in both scopes, including container builds.
Profiles select files; they never automatically start daemons. The local archive
installer follows ordinary successful installation with `anolisa setup`: unattended use
prints an overview, and a terminal can guide explicit invoking-user configuration.
`anolisa overview` reports recorded component versions and local availability;
`--candidate-bundle DIR` compares a verified local release without upgrading it.
The website's explicit `--rpm` route uses `install --all --backend rpm` in system
mode for the full published native component set. An RPM wrapper
for the same profile can be generated with:

```bash
python3 distribution/packaging/product.py --format rpm-spec \
  --profile standard-system --version PRODUCT_VERSION --output /tmp/anolisa-standard.spec
```

The generated noarch package depends on real component subpackages. The
`anolisa-policy` RPM supplies explicit disabled service defaults; administrators
apply a preset separately. See the [installation guide](../docs/user-guide/en/getting-started/installation.md)
for offline installation, profiles and recorded migration.

After installing a system tarball, run these commands as root to review and
publish the selected backends' existing unit templates:

```bash
anolisa release services --bundle /absolute/unpacked-bundle --scope system \
  --service agent-sec-core --service agentsight --service ws-ckpt --json
anolisa release services --bundle /absolute/unpacked-bundle --scope system \
  --service agent-sec-core --service agentsight --service ws-ckpt --apply
anolisa preset plan --preset host --scope system
anolisa preset apply --preset host --scope system
```

Use the same profile or component selection as the product installation.
Unit publication verifies installed files, binds programs to their selected
version, refuses unknown existing units and reloads systemd. It does not start
or enable services. Preset application preserves existing boot-enable choices;
`--initialize` separately asks systemd to apply administrator-aware preset policy
for units not previously initialized. Component configuration keeps its existing
location and defaults. Native RPMs already publish their units. COSH's system
gateway requires its own containment and instance setup and is not registered
by this generic template publisher.

## Containers

Place a verified, unpacked release at `bundle/` in the repository build context:

```bash
docker build -f distribution/containers/Containerfile -t anolisa:local .
docker run --rm -it \
  --mount type=bind,source=/absolute/aw-config,target=/home/anolisa/.config/aw,readonly \
  anolisa:local
```

The image installs `complete` as UID 10001, copies installed files to the
final stage, and runs `anolisa serve --preset container`. Supply an actual
`aw.yaml` in the mounted configuration directory. Programs, state and resource
paths are identical between installation and final stages. `ANOLISA_PROFILE`
can choose another user-compatible package profile at image build time.
The image uses Ubuntu 24.04; provide GNU binaries compatible with that runtime.
Installation during image construction rejects an incompatible AW or CLI binary.

The container preset currently starts AW only. Privileged backend daemons,
FUSE, eBPF and systemd containment are not silently enabled. Providers and MCP
stdio servers keep their per-session lifecycle. The supervisor waits for
readiness, forwards termination signals, stops dependent processes and reaps
children under `tini`; systemd remains the host service manager.

## Validation

```bash
python3 -m unittest discover -s distribution/anolisa/packaging/release/tests -v
bash website/scripts/test-install.sh
cd distribution/anolisa
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo doc --workspace --no-deps --locked
```

Packaging fixtures exercise already built bytes on macOS as well as Linux.
Real AW installation, RPM transactions and container acceptance require their
supported Linux environments; do not build Linux-only components on macOS.
