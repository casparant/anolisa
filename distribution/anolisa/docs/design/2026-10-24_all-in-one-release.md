# Complete product release contract

[中文版](2026-10-24_all-in-one-release_zh.md)

ANOLISA builds one Linux archive from expanded AW packages. ANOLISA owns the
composition, download sources, product records and activation. AW verifies and
commits local package files through its existing database and transactions.

## Build and archive

[`packaging/release/linux.json`](../../packaging/release/linux.json) defines the
complete composition, required resources, dependencies, build adapters and public
commands. It selects cosh-ng and excludes the conflicting historical cosh.
Backends and Provider bridges have disjoint file ownership; Provider declarations
do not enable instances or grant execution permission.

Source mode requires a clean Linux x86_64 checkout and the existing prepared
release Cross profiles. It fixes a detached source revision and isolates build
worktrees, Cargo targets, logs and outputs. Staged mode requires the complete
explicit package set and does not imply that components were built or run.

The assembler checks descriptions, versions, digests, permissions, ELF
architecture and GNU loader compatibility, exact dependencies, file conflicts,
resources and AW capacity limits. Source provenance records native libc and ELF
dependencies; `abi = "gnu"` does not promise a common minimum glibc version for
every build host. Tar order, ownership, timestamps and gzip metadata are
normalized. Publishing requires a new output directory and preserves existing
outputs.

The archive contains `release.json`, `install.sh`, per-file checksums,
provenance and `packages/<id>/{package.json,payload/}`. Licenses travel with
package resources. The launcher uses the bundled ANOLISA executable directly;
component tarballs and duplicate bootstrap binaries are unnecessary.

## Metadata v1

`release.json` requires `schema_version`, `product_version`, `release_id`,
`source_commit`, `os`, `arch`, `abi`, `min_installer_version`, `packages` and
`entrypoints`. Unknown fields and unsupported schemas are rejected.

Each package reference records `package_id`, its independent `version`, a
relative `path` and `description_sha256`. Exact dependencies must appear earlier
in the package list. `entrypoints` maps public command names to executable
payload paths. A release ID binds immutable manifest and package content.

An explicit HTTPS artifact base URL also generates a product download index
with `schema_version`, `product_version` and `artifacts`, whose fields are
`os`, `arch`, `abi`, `url`, `sha256` and `size`. This JSON protocol is separate
from the existing TOML CLI self-update manifest. Checksums establish content
integrity, not independent publisher identity.

## Installation and recovery

The bundled CLI verifies the whole bundle before invoking its fixed AW binary.
Each package uses a separate AW transaction. Product records retain actual AW
commit and recovery facts. Identical-release resume verifies descriptions and
installed bytes before skipping healthy packages; changed or ambiguous content
is refused.

| Scope | Private program root | Resource root | Public commands |
| --- | --- | --- | --- |
| User | `~/.local/lib/<arch>-linux-gnu/anolisa/releases/<id>` | `$XDG_DATA_HOME/anolisa/resources/releases/<arch>-linux-gnu/<id>` | `~/.local/bin` |
| System tarball | `/usr/local/lib/anolisa/releases/<id>` | `/usr/local/share/anolisa/releases/<id>` | `/usr/local/bin` |

User templates, operation records and download cache follow their XDG roots.
System templates use `/usr/local/etc/anolisa`, operation records use
`/var/lib/anolisa`, and product cache uses `/var/cache/anolisa`. Ordinary runtime
users retain their own XDG directories regardless of software installation
scope. Explicit system services require separate service identities and paths.

AW databases remain `$XDG_STATE_HOME/aw/packages` and `/var/lib/aw/packages`.
The installer passes the resolved absolute state root to AW. Legacy
`FsLayout::system(prefix)` is not used for release paths. AW receives the final
real prefix, never `current` or a directory renamed after registration.
`aw install --resources-dir` records the immutable external resource root;
logical `share/` paths resolve there, while `bin/` and `libexec/` stay in the
program prefix. Inventory, doctor, uninstall and recovery use the recorded
mapping. Programs and resources stage locally on their own filesystems. Required
filesystem operations are probed before payload commits. ANOLISA records and
checks the program prefix's `share` compatibility link to the same resource root;
runtime components can retain their existing relative resource lookup.

Unknown existing command entries are preserved and reported before payload
installation. Required package and configuration work precedes publishing
external launchers and the current-version reference. Launchers resolve and pin
one real version; user launchers also select the host architecture. Package
commits, product records and activation are separate durable operations, so
reports can contain committed packages with an inactive product. They do not
claim a release-wide atomic rollback.

Disabled Provider templates retain fixed program/resource paths and their
generated digest across identical-release retries. They describe invoking-user
XDG resolution rather than capturing the installer's data or socket paths;
rendering them into real AW transport argv is a separate explicit operation.
AW does not interpolate shell variables in argv. Session changes do not change
the template bytes, while user edits require explicit recovery.
Provider templates are prepared without activation. File installation does
not imply service startup, capabilities, or historical data migration. Initial
support covers fresh installation and identical-release resume. Taking over old
product data requires an explicit migration operation.

## Compatibility and validation

New default Linux downloads use the product protocol. Explicit legacy options
and CLI environment overrides preserve their original route and meanings;
macOS keeps standalone CLI downloads. Mixed input groups are refused. Product
failure never silently falls back to another installer. The standalone updater
refuses AW registration or lock evidence before fetching or replacing the CLI;
existing RPM delegation remains intact.

Assembly tests use synthetic complete compositions. The separate Linux smoke
test uses real AW and ANOLISA with a small composition, including cross-filesystem
user resources, mapped-resource doctor/uninstall, root install followed by
non-root public-command execution and queries, resume and tamper refusal. These are
different verification scopes. Full source construction runs through the
explicit `full_build` CI input on the prepared release runner.
