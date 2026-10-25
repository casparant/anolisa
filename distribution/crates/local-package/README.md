# Local package library

[中文版](README_zh.md)

Local package verification, file ownership and durable installation transactions
for ANOLISA and AW extension compatibility. This Rust library has no CLI and does
not select repositories, product versions, profiles or services. Callers choose
the manager and database explicitly.

| Caller | User database | System database |
|--------|---------------|-----------------|
| ANOLISA product raw | `$XDG_STATE_HOME/anolisa/packages` | `/var/lib/anolisa/packages` |
| AW local extensions and legacy Preview | `$XDG_STATE_HOME/aw/packages` | `/var/lib/aw/packages` |

User paths fall back to `$HOME/.local/state`. `DatabasePaths::user_for` and
`DatabasePaths::system_for` select `PackageManager::Aw` or
`PackageManager::Anolisa`. `open_bundle` verifies local input; `plan` derives
read-only installation/removal plans; `transaction` commits or recovers recorded
file changes. Queries and plans do not create database files.

The historical `.aw-packages/aw-core.json` locator filename and runtime-lock
markers remain compatible. New product locators identify their manager; an old
locator without that field means AW. A caller must not follow another manager's
locator to gain write or removal authority. Verified existing AW-owned product
installations may retain their original authority; no automatic transfer or
duplicate ownership is performed. RPM and dpkg retain native file ownership.

`runtime/` contains the smaller `local-package-runtime` library for file leases
and lock discovery. AW runtime code uses it without reading either package
database or depending on the transaction engine.

Run the library's tests on Linux; its file transactions require Linux filesystem
operations. See the [local package design](../../../src/aw/docs/design/en/packaging/package-management.md)
for limits, lock ordering and recovery rules, and the
[distribution guide](../../README.md) for product packaging.
