#!/usr/bin/env python3
"""Adapt existing component release payloads to one legal AW-owned prefix."""

from __future__ import annotations

import shutil
import stat
import tarfile
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[4]))
from distribution.packaging.staging import normalize, split

from release_common import (
    MAX_PREFIX_BYTES,
    MAX_PREFIX_FILES,
    digest,
    files,
    json_bytes,
    regular,
    relative,
    require,
)


def copy_file(source: Path, destination: Path, executable: bool | None = None) -> None:
    """Preserve ordinary content, normalizing the two supported payload modes."""
    metadata = regular(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    require(not destination.exists(), f"staging ownership conflict: {destination}")
    shutil.copyfile(source, destination, follow_symlinks=False)
    execute = bool(metadata.st_mode & 0o111) if executable is None else executable
    destination.chmod(0o755 if execute else 0o644)


def copy_tree(source: Path, destination: Path) -> None:
    """Copy complete runtime resource directories without following symlinks."""
    for name, path in files(source).items():
        copy_file(path, destination / name)


def unpack_raw(archive: Path, destination: Path) -> None:
    """Extract only bounded regular raw-package members; never run package scripts."""
    regular(archive, MAX_PREFIX_BYTES)
    destination.mkdir(parents=True, exist_ok=False)
    names: set[str] = set()
    count = 0
    size = 0
    with tarfile.open(archive, "r:gz") as source:
        for member in source:
            name = member.name
            while name.startswith("./"):
                name = name[2:]
            if name in {"", "."}:
                require(member.isdir(), "non-directory archive root")
                continue
            name = relative(name.rstrip("/") if member.isdir() else name)
            require(name not in names, f"duplicate archive member: {name}")
            names.add(name)
            require(member.isdir() or member.isfile(), f"unsupported archive member: {name}")
            target = destination / name
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                target.chmod(0o755)
                continue
            count += 1
            size += member.size
            require(
                0 <= member.size <= 1024**3
                and count <= MAX_PREFIX_FILES
                and size <= MAX_PREFIX_BYTES,
                "raw input exceeds release budget",
            )
            target.parent.mkdir(parents=True, exist_ok=True)
            stream = source.extractfile(member)
            require(stream is not None, f"missing archive content: {name}")
            with stream, target.open("xb") as output:
                shutil.copyfileobj(stream, output, 1024**2)
            require(target.stat().st_size == member.size, f"short archive member: {name}")
            target.chmod(0o755 if member.mode & 0o111 else 0o644)


def mapped_path(package_id: str, name: str) -> str:
    """Map known component layouts explicitly; unsupported roots fail closed."""
    if name == ".anolisa/component.toml":
        return f"share/anolisa/components/{package_id}/source-component.toml"
    if package_id == "anolisa-cli" and name == "anolisa":
        return "bin/anolisa"
    if name.startswith(("bin/", "libexec/", "share/")):
        return name
    if name.startswith("adapters/"):
        suffix = name.removeprefix("adapters/")
        if package_id == "tokenless":
            suffix = "tokenless/" + suffix
        return "share/anolisa/adapters/" + suffix
    if package_id == "tokenless" and name.startswith("extensions/"):
        return "share/anolisa/" + name
    if package_id == "sec-core" and name.startswith("lib/anolisa/sec-core/"):
        return "libexec/agent-sec-core/" + name.removeprefix("lib/anolisa/sec-core/")
    if package_id == "sec-core" and name.startswith("lib/agent-sec-core/"):
        return "libexec/agent-sec-core/" + name.removeprefix("lib/agent-sec-core/")
    raise ValueError(f"unsupported {package_id} raw payload path: {name}")


def rewrite(path: Path, before: str, after: str) -> None:
    """Require the expected source contract before rewriting a release launcher."""
    content = path.read_text(encoding="utf-8")
    require(before in content, f"launcher layout changed; review release adapter: {path}")
    path.write_text(content.replace(before, after), encoding="utf-8")


def adapt_launchers(payload: Path, package_id: str) -> None:
    """Keep child executables bound to this release rather than the host PATH."""
    if package_id == "sec-core":
        launcher = payload / "bin/agent-sec-python"
        content = launcher.read_text(encoding="utf-8")
        origins = ("$PREFIX/lib/agent-sec-core/", "$PREFIX/lib/anolisa/sec-core/")
        require(any(origin in content for origin in origins),
                f"unknown sec-core Python launcher layout: {launcher}")
        for origin in origins:
            content = content.replace(origin, "$PREFIX/libexec/agent-sec-core/")
        launcher.write_text(content, encoding="utf-8")
    elif package_id == "tokenless":
        binary = payload / "bin/tokenless"
        private = payload / "libexec/anolisa/tokenless/tokenless"
        require(binary.is_file() and not private.exists(), "unexpected Tokenless raw binary layout")
        private.parent.mkdir(parents=True, exist_ok=True)
        binary.rename(private)
        binary.write_text(
            "#!/bin/sh\nset -eu\n"
            'prefix=$(CDPATH=\'\' cd -P -- "$(dirname -- "$0")/.." && pwd -P)\n'
            'export PATH="$prefix/bin:$PATH"\n'
            'export TOKENLESS_TOOL_READY_SPEC="$prefix/share/anolisa/adapters/tokenless/common/tool-ready-spec.json"\n'
            'export TOKENLESS_ENV_FIX_SCRIPT="$prefix/share/anolisa/adapters/tokenless/common/tokenless-env-fix.sh"\n'
            'exec "$prefix/libexec/anolisa/tokenless/tokenless" "$@"\n',
            encoding="utf-8",
        )
        binary.chmod(0o755)
        launcher = payload / "bin/rtk"
        require(not launcher.exists(), "Tokenless raw package unexpectedly owns bin/rtk")
        launcher.write_text(
            "#!/bin/sh\nset -eu\n"
            'prefix=$(CDPATH=\'\' cd -P -- "$(dirname -- "$0")/.." && pwd -P)\n'
            'exec "$prefix/libexec/anolisa/tokenless/rtk" "$@"\n',
            encoding="utf-8",
        )
        launcher.chmod(0o755)
    elif package_id == "agentsight":
        launcher = payload / "bin/agentsight-start"
        rewrite(
            launcher,
            "start_workers() {",
            "BINDIR=$(CDPATH='' cd -P -- " '"$(dirname -- "$0")" && pwd -P)\n\nstart_workers() {',
        )
        rewrite(launcher, "    agentsight trace &", '    "$BINDIR/agentsight" trace &')
        rewrite(launcher, "    agentsight serve --host", '    "$BINDIR/agentsight" serve --host')
    elif package_id == "ws-ckpt":
        for framework in ("openclaw", "hermes"):
            helper = payload / "share/anolisa/adapters/ws-ckpt" / framework / "lib-discover.sh"
            if not helper.exists():
                helper = payload / "share/ws-ckpt/adapters" / framework / "lib-discover.sh"
            datadir = (
                "WS_CKPT_RELEASE_DATADIR=$(CDPATH='' cd -P -- "
                '"$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)\n\n'
            )
            rewrite(
                helper,
                "# discover_dir REQUIRED_FILE... -- DIR...",
                datadir + "# discover_dir REQUIRED_FILE... -- DIR...",
            )
            rewrite(
                helper,
                '    local component="${1:?usage: find_plugin_src COMPONENT}"\n'
                "    local candidates=()",
                '    local component="${1:?usage: find_plugin_src COMPONENT}"\n'
                '    local candidates=("${WS_CKPT_RELEASE_DATADIR}/runtime/ws-ckpt/plugins/${component}")',
            )
            rewrite(
                helper,
                "find_skill_src() {\n    local candidates=()",
                'find_skill_src() {\n    local candidates=("${WS_CKPT_RELEASE_DATADIR}/runtime/skills/ws-ckpt")',
            )
    elif package_id == "cosh-ng":
        # This archive deliberately excludes the conflicting historical cosh package.
        # A raw release must never swap unrelated RPM-owned host binaries.
        launcher = payload / "bin/cosh-switch"
        launcher.write_text(
            "#!/bin/sh\n"
            'printf "%s\\n" "This ANOLISA release includes cosh-ng. '
            "Changing the shell implementation requires a different product release; "
            'cosh-switch does not modify host RPM packages." >&2\nexit 1\n',
            encoding="utf-8",
        )
        launcher.chmod(0o755)


def adapt_units(payload: Path, package_id: str) -> None:
    """Keep units as explicit templates for separately authorized host integration."""
    substitutions = {
        "agentsight": [
            (
                "share/anolisa/agentsight/agentsight.service",
                "ExecStart=/usr/local/bin/agentsight-start",
                'ExecStart="{bindir}/agentsight-start"',
            ),
            (
                "share/anolisa/agentsight/agentsight-enforcer.service",
                "ExecStart=/usr/local/bin/agentsight-enforcer",
                'ExecStart="{bindir}/agentsight-enforcer"',
            ),
        ],
        "agent-memory": [
            (
                "share/anolisa/agent-memory/anolisa-memory@.service",
                "ExecStart=/usr/bin/agent-memory serve",
                'ExecStart="{bindir}/agent-memory" serve',
            ),
        ],
        "ws-ckpt": [
            (
                "share/anolisa/ws-ckpt/ws-ckpt.service",
                "ExecStart=/usr/local/bin/ws-ckpt daemon",
                'ExecStart="{bindir}/ws-ckpt" daemon',
            ),
            (
                "share/anolisa/ws-ckpt/ws-ckpt.service",
                "ExecReload=/usr/local/bin/ws-ckpt reload",
                'ExecReload="{bindir}/ws-ckpt" reload',
            ),
        ],
    }
    for name, before, after in substitutions.get(package_id, []):
        path = payload / name
        if not path.exists():
            namespace = package_id
            path = payload / "share" / namespace / "templates" / Path(name).name
        rewrite(path, before, after)


def notices(source: Path, payload: Path, package_id: str, component: Path) -> None:
    """Carry component licensing without inventing a separate runtime owner."""
    for name in ("LICENSE", "NOTICE"):
        target = payload / "share/doc" / package_id / name
        if not target.exists():
            candidate = component / name if (component / name).is_file() else source / name
            if candidate.is_file():
                resolved = candidate.resolve(strict=True)
                require(
                    resolved.is_relative_to(source.resolve()),
                    f"license points outside the pinned source: {candidate}",
                )
                copy_file(resolved, target, executable=False)


def raw_package(raw: Path, target: Path, spec: dict[str, Any], source: Path) -> None:
    """Adapt one already-built release payload; compilation stays in its component."""
    package_id = spec["package_id"]
    payload = target / "payload"
    payload.mkdir(parents=True)
    for name, path in files(raw).items():
        copy_file(path, payload / mapped_path(package_id, name))
    adapt_launchers(payload, package_id)
    adapt_units(payload, package_id)
    if package_id == "anolisa-cli":
        for name in ("templates", "manifests"):
            copy_tree(source / spec["source"] / name, payload / "share/anolisa" / name)
        copy_file(source / "distribution/release/services.json", payload / "share/anolisa/distribution/services.json", False)
    notices(source, payload, package_id, source / spec["source"])


def split_component(
    recipe: dict[str, Any], component_payload: Path, packages: Path, canonical: Path,
    source: Path,
) -> list[str]:
    """Apply one content recipe to independent and all-in-one local packages."""
    normalize(recipe, component_payload, canonical)
    if recipe["id"] == "tokenless":
        launcher = canonical / "bin/tokenless"
        content = launcher.read_text(encoding="utf-8")
        launcher.write_text(content.replace("share/anolisa/adapters/tokenless/", "share/tokenless/adapters/"), encoding="utf-8")
    if recipe["id"] == "ws-ckpt":
        for framework in ("openclaw", "hermes"):
            helper = canonical / "share/ws-ckpt/adapters" / framework / "lib-discover.sh"
            content = helper.read_text(encoding="utf-8")
            content = content.replace('"${BASH_SOURCE[0]}")/../../..', '"${BASH_SOURCE[0]}")/../..')
            content = content.replace("${WS_CKPT_RELEASE_DATADIR}/runtime/ws-ckpt/plugins/", "${WS_CKPT_RELEASE_DATADIR}/adapters/")
            content = content.replace("${WS_CKPT_RELEASE_DATADIR}/runtime/skills/", "${WS_CKPT_RELEASE_DATADIR}/skills/")
            helper.write_text(content, encoding="utf-8")
    license_file = source / recipe["build"]["source"] / "LICENSE"
    if not license_file.is_file():
        license_file = source / "LICENSE"
    return split(recipe, canonical, packages, license_file.resolve(strict=True))


def cargo_package(binary: Path, target: Path, spec: dict[str, Any], source: Path) -> None:
    """Stage framework or bridge binaries with their actual private runtime resources."""
    package_id = spec["package_id"]
    payload = target / "payload"
    payload.mkdir(parents=True)
    if spec.get("provider"):
        provider = "tokenless" if package_id == "aw-provider-tokenless" else "sec-core"
        copy_file(binary, payload / "libexec/aw/providers" / provider / spec["binary"], True)
        for directory in ("schemas", "examples"):
            origin = source / spec["source"] / directory
            if origin.is_dir():
                copy_tree(origin, payload / "share/aw/providers" / provider / directory)
        if provider == "sec-core":
            for name in ("agent-sec-core-v2.service.in", "skillsec.json"):
                copy_file(
                    source / "src/agent-sec-core/packaging/systemd" / name,
                    payload / "share/aw/providers/sec-core" / name,
                    False,
                )
    else:
        copy_file(binary, payload / "bin" / spec["binary"], True)
    if package_id == "aw-core":
        for crate in sorted((source / "src/aw/crates").iterdir()):
            if (crate / "schemas").is_dir():
                copy_tree(crate / "schemas", payload / "share/aw/schemas" / crate.name)
        copy_tree(source / "src/aw/crates/aw-adapter/native", payload / "share/aw/native")
    notices(source, payload, package_id, source / spec["source"])


def write_description(
    target: Path, spec: dict[str, Any], versions: dict[str, str], arch: str, source_commit: str
) -> None:
    """Bind the staged bytes, independent version and exact local dependencies."""
    inventory = {
        name: {"sha256": digest(path), "mode": stat.S_IMODE(path.stat().st_mode)}
        for name, path in files(target / "payload").items()
    }
    description: dict[str, Any] = {
        "schema_version": 1,
        "package_id": spec["package_id"],
        "version": versions[spec["package_id"]],
        "os": "linux",
        "arch": arch,
        "files": inventory,
        "dependencies": [
            {"package_id": name, "version": versions[name]} for name in spec["dependencies"]
        ],
        "source_commit": source_commit,
    }
    if spec.get("provider"):
        description["provider"] = {"protocol": "aw-provider/v1alpha1"}
    path = target / "package.json"
    path.write_bytes(json_bytes(description))
    path.chmod(0o644)
