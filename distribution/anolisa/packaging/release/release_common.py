#!/usr/bin/env python3
"""Validate release inputs independently of the host's installed software."""

from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import struct
from pathlib import Path, PurePosixPath
from typing import Any

import sys

SOURCE_ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(SOURCE_ROOT))
from distribution.packaging.recipes import package_specs

MAX_FILE_BYTES = 1024**3
MAX_PREFIX_BYTES = 4 * MAX_FILE_BYTES
MAX_PACKAGE_FILES = 16384
MAX_PREFIX_FILES = 32767
MAX_DIRECTORIES = 32768
MAX_DESCRIPTION_BYTES = 1024**2
MAX_DATABASE_BYTES = 16 * 1024**2
MAX_JOURNAL_BYTES = 96 * 1024**2
IDENTIFIER = re.compile(r"[a-z0-9][a-z0-9._-]{0,127}\Z")
SEMVER = re.compile(
    r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?\Z"
)
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
REQUIRED_PACKAGE_FIELDS = {
    "schema_version",
    "package_id",
    "version",
    "os",
    "arch",
    "files",
    "dependencies",
}
OPTIONAL_PACKAGE_FIELDS = {"source_commit", "provider"}


class ReleaseError(ValueError):
    """An input cannot safely represent the declared complete release."""


def require(condition: bool, message: str) -> None:
    """Reject an invalid input with its concrete reason."""
    if not condition:
        raise ReleaseError(message)


def version(value: Any) -> str:
    """Validate the exact SemVer accepted by the local package contract."""
    require(isinstance(value, str) and len(value) <= 128, "version must be an exact SemVer")
    match = SEMVER.fullmatch(value)
    require(match is not None, f"invalid exact SemVer: {value!r}")
    if match and match[4]:
        require(
            all(
                not part.isdigit() or part == "0" or not part.startswith("0")
                for part in match[4].split(".")
            ),
            f"numeric prerelease identifier has a leading zero: {value}",
        )
    return value


def identifier(value: Any) -> str:
    """Restrict product and package identities to one safe path component."""
    require(
        isinstance(value, str) and IDENTIFIER.fullmatch(value) is not None,
        f"invalid identifier: {value!r}",
    )
    return value


def relative(value: Any, payload: bool = False) -> str:
    """Reject ambiguous paths before using them for reads or archive members."""
    require(isinstance(value, str) and bool(value), "path must be a nonempty string")
    require(
        not any(ord(char) < 32 or ord(char) == 127 for char in value),
        f"control character in path: {value!r}",
    )
    parts = value.split("/")
    require(
        not value.startswith("/")
        and "\\" not in value
        and all(part not in {"", ".", ".."} for part in parts),
        f"invalid relative path: {value!r}",
    )
    require(
        len(value.encode()) <= 4095 and all(len(part.encode()) <= 255 for part in parts),
        f"path exceeds filesystem bounds: {value!r}",
    )
    if payload:
        require(
            parts[0] in {"bin", "libexec", "share"} and len(parts) > 1,
            f"unsupported payload root: {value}",
        )
    return value


def no_links(path: Path) -> None:
    """Reject symlinks in input roots as well as their parents."""
    for parent in (path, *path.parents):
        require(not parent.is_symlink(), f"symbolic link in input path: {parent}")


def regular(path: Path, limit: int = MAX_FILE_BYTES) -> os.stat_result:
    """Check a bounded ordinary file without accepting symlinks or special files."""
    no_links(path)
    metadata = path.lstat()
    require(stat.S_ISREG(metadata.st_mode), f"not a regular file: {path}")
    require(metadata.st_size <= limit, f"file exceeds {limit} bytes: {path}")
    return metadata


def digest(path: Path) -> str:
    """Hash bounded content without loading a runtime archive into memory."""
    metadata = regular(path)
    result = hashlib.sha256()
    count = 0
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024**2), b""):
            count += len(chunk)
            require(count <= MAX_FILE_BYTES, f"file grew beyond limit: {path}")
            result.update(chunk)
    require(count == metadata.st_size, f"file changed while hashing: {path}")
    return result.hexdigest()


def json_bytes(value: Any) -> bytes:
    """Emit stable UTF-8 metadata without host paths or timestamps."""
    return (
        json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n"
    ).encode()


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    """Refuse duplicate fields instead of letting JSON silently choose a value."""
    result: dict[str, Any] = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON field: {key}")
        result[key] = value
    return result


def read_json(path: Path, limit: int = MAX_DESCRIPTION_BYTES) -> dict[str, Any]:
    """Read one bounded JSON object with duplicate-key detection."""
    regular(path, limit)
    with path.open("rb") as stream:
        raw = stream.read(limit + 1)
    require(len(raw) <= limit, f"JSON exceeds {limit} bytes: {path}")
    result = json.loads(raw, object_pairs_hook=unique_object)
    require(isinstance(result, dict), f"expected JSON object: {path}")
    return result


def files(root: Path) -> dict[str, Path]:
    """Enumerate only real directories and ordinary files, including dotfiles."""
    no_links(root)
    require(root.is_dir(), f"missing input directory: {root}")
    result: dict[str, Path] = {}
    for directory, names, filenames in os.walk(root, followlinks=False):
        parent = Path(directory)
        for name in names:
            path = parent / name
            require(stat.S_ISDIR(path.lstat().st_mode), f"not an ordinary directory: {path}")
        for name in filenames:
            path = parent / name
            regular(path)
            result[relative(path.relative_to(root).as_posix())] = path
    return dict(sorted(result.items()))


def elf(path: Path, arch: str, executable: bool) -> None:
    """Check every ELF's machine and reject foreign executable formats."""
    with path.open("rb") as stream:
        header = stream.read(64)
    if header.startswith(b"\x7fELF"):
        require(
            len(header) >= 64 and header[4:7] == b"\x02\x01\x01",
            f"expected 64-bit little-endian ELF: {path}",
        )
        machine = struct.unpack_from("<H", header, 18)[0]
        kind = struct.unpack_from("<H", header, 16)[0]
        # eBPF object code is a resource consumed by the host, not a host executable.
        bpf_resource = machine == 247 and kind == 1 and path.suffix == ".o" and not executable
        require(
            bpf_resource or machine == {"x86_64": 62, "aarch64": 183}[arch],
            f"ELF architecture does not match {arch}: {path}",
        )
        require(not executable or kind in {2, 3}, f"ELF is not executable: {path}")
        offset = struct.unpack_from("<Q", header, 32)[0]
        width, count = struct.unpack_from("<HH", header, 54)
        if count:
            require(
                width >= 56 and count <= 4096 and offset + width * count <= path.stat().st_size,
                f"invalid ELF program headers: {path}",
            )
            with path.open("rb") as stream:
                for index in range(count):
                    stream.seek(offset + index * width)
                    program = stream.read(56)
                    if struct.unpack_from("<I", program)[0] != 3:
                        continue
                    position = struct.unpack_from("<Q", program, 8)[0]
                    size = struct.unpack_from("<Q", program, 32)[0]
                    require(
                        0 < size <= 4096 and position + size <= path.stat().st_size,
                        f"invalid ELF interpreter: {path}",
                    )
                    stream.seek(position)
                    interpreter = stream.read(size).rstrip(b"\0")
                    loader = {
                        "x86_64": b"ld-linux-x86-64.so.2",
                        "aarch64": b"ld-linux-aarch64.so.1",
                    }[arch]
                    require(
                        interpreter.endswith(b"/" + loader),
                        f"ELF interpreter is not the declared GNU ABI: {path}",
                    )
    elif executable:
        require(header.startswith(b"#!"), f"executable is neither ELF nor a script: {path}")
    elif header[:4] in {b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"MZ\x90\x00"}:
        # Python packaging tools carry Windows launcher templates as inert data.
        if (
            not executable
            and (
                "/site-packages/pip/_vendor/distlib/" in str(path)
                and path.name in {"t32.exe", "t64.exe", "t64-arm.exe", "w32.exe", "w64.exe", "w64-arm.exe"}
                or "/site-packages/setuptools/" in str(path)
                and path.name in {"cli.exe", "cli-32.exe", "cli-64.exe", "cli-arm64.exe", "gui.exe", "gui-32.exe", "gui-64.exe", "gui-arm64.exe"}
            )
        ):
            return
        raise ReleaseError(f"foreign binary format in Linux payload: {path}")


def composition(path: Path) -> list[dict[str, Any]]:
    """Read the checked-in complete Linux composition, preserving its stable order."""
    document = read_json(path)
    if document.get("schema_version") == 2:
        require(document.get("recipes") == "distribution/components", "unsupported recipe location")
        return package_specs(SOURCE_ROOT)
    require(document.get("schema_version") == 1, "unsupported composition schema")
    packages = document.get("packages")
    require(isinstance(packages, list) and bool(packages), "empty composition")
    seen: set[str] = set()
    for package in packages:
        package_id = identifier(package["package_id"])
        require(package_id not in seen, f"duplicate composition package: {package_id}")
        seen.add(package_id)
        for path_value in package["required"]:
            relative(path_value.rstrip("/"), payload=True)
    return packages


def validate_package(
    root: Path, spec: dict[str, Any], arch: str, source_commit: str
) -> dict[str, Any]:
    """Validate the exact description, payload, resource set and ELF architecture."""
    document = read_json(root / "package.json")
    require(
        REQUIRED_PACKAGE_FIELDS <= document.keys()
        and document.keys() <= REQUIRED_PACKAGE_FIELDS | OPTIONAL_PACKAGE_FIELDS,
        f"unexpected package description fields: {root}",
    )
    package_id = spec["package_id"]
    require(
        type(document["schema_version"]) is int
        and document["schema_version"] == 1
        and document["package_id"] == package_id,
        f"package identity mismatch: {root}",
    )
    version(document["version"])
    require(
        document["os"] == "linux" and document["arch"] == arch,
        f"package platform mismatch: {package_id}",
    )
    require(
        document.get("source_commit") == source_commit,
        f"package source_commit mismatch: {package_id}",
    )
    if spec.get("provider"):
        require(
            document.get("provider") == {"protocol": "aw-provider/v1alpha1"},
            f"missing Provider protocol: {package_id}",
        )
    else:
        require("provider" not in document, f"ordinary package declares a Provider: {package_id}")
    require(
        set(path.name for path in root.iterdir()) == {"package.json", "payload"},
        f"unexpected files next to package.json: {root}",
    )
    payload = document["files"]
    require(
        isinstance(payload, dict) and 0 < len(payload) <= MAX_PACKAGE_FILES,
        f"invalid package file count: {package_id}",
    )
    actual = files(root / "payload")
    require(actual.keys() == payload.keys(), f"payload inventory mismatch: {package_id}")
    for name, metadata in payload.items():
        relative(name, payload=True)
        require(
            isinstance(metadata, dict) and metadata.keys() == {"sha256", "mode"},
            f"invalid payload metadata: {package_id}/{name}",
        )
        require(
            isinstance(metadata["sha256"], str) and SHA256.fullmatch(metadata["sha256"]),
            f"invalid digest: {package_id}/{name}",
        )
        mode = metadata["mode"]
        require(
            type(mode) is int and mode in {0o644, 0o755},
            f"unsupported payload mode: {package_id}/{name}",
        )
        require(
            stat.S_IMODE(actual[name].stat().st_mode) == mode,
            f"payload mode mismatch: {package_id}/{name}",
        )
        require(
            digest(actual[name]) == metadata["sha256"],
            f"payload digest mismatch: {package_id}/{name}",
        )
        elf(actual[name], arch, mode == 0o755 and name.startswith(("bin/", "libexec/")))
        if spec.get("provider") and name.startswith(("bin/", "libexec/")):
            bridge = "libexec/aw/providers/" + (
                "tokenless" if package_id == "aw-provider-tokenless" else "sec-core"
            )
            require(
                name == bridge + "/" + package_id,
                f"Provider package duplicates a backend executable: {package_id}/{name}",
            )
    for name in spec["required"]:
        if name.endswith("/"):
            require(any(path.startswith(name) for path in payload), f"missing required release directory: {package_id}/{name}")
            continue
        require(
            name in payload and actual[name].stat().st_size > 0,
            f"missing required release resource: {package_id}/{name}",
        )
    dependencies = document["dependencies"]
    require(isinstance(dependencies, list), f"invalid dependencies: {package_id}")
    selected: set[str] = set()
    for dependency in dependencies:
        require(
            isinstance(dependency, dict) and dependency.keys() == {"package_id", "version"},
            f"invalid exact dependency: {package_id}",
        )
        dep_id = identifier(dependency["package_id"])
        version(dependency["version"])
        require(
            dep_id != package_id and dep_id not in selected,
            f"duplicate or self dependency: {package_id}/{dep_id}",
        )
        selected.add(dep_id)
    require(set(spec["dependencies"]) <= selected, f"missing composition dependency: {package_id}")
    return document


def check_budget(packages: list[dict[str, Any]], sizes: dict[str, int]) -> None:
    """Reserve locator space and conservative metadata capacity for one AW prefix."""
    require(len(sizes) <= MAX_PREFIX_FILES, "complete prefix exceeds 32767 payload files")
    require(sum(sizes.values()) <= MAX_PREFIX_BYTES, "complete prefix exceeds 4 GiB")
    parents = {
        str(parent)
        for name in sizes
        for parent in PurePosixPath(name).parents
        if str(parent) != "."
    } | {".aw-packages"}
    require(len(parents) <= MAX_DIRECTORIES, "complete prefix exceeds journal directory count")
    # This bound deliberately reserves large identities, both maximum snapshots,
    # escaped paths and per-action metadata. The target installer repeats its exact check.
    payload_metadata = len(json_bytes(packages))
    require(
        payload_metadata + 16384 <= MAX_DATABASE_BYTES,
        "complete package metadata exceeds the 16 MiB database budget",
    )
    path_bytes = sum(len(json.dumps(name).encode()) for name in sizes)
    directory_bytes = sum(len(json.dumps(name).encode()) for name in parents)
    journal_bound = 4 * MAX_DATABASE_BYTES + 4 * path_bytes + 512 * (len(sizes) + 1)
    journal_bound += 4 * directory_bytes + 256 * len(parents) + 65536
    require(
        journal_bound <= MAX_JOURNAL_BYTES,
        "complete prefix exceeds conservative 96 MiB journal budget",
    )


def validate_packages(
    packages_dir: Path, specs: list[dict[str, Any]], arch: str, source_commit: str
) -> list[dict[str, Any]]:
    """Check the complete composition and return a deterministic dependency order."""
    require(arch in {"x86_64", "aarch64"}, f"unsupported architecture: {arch}")
    require(COMMIT.fullmatch(source_commit) is not None, "source_commit must be 40 lowercase hex")
    no_links(packages_dir)
    require(packages_dir.is_dir(), f"missing packages directory: {packages_dir}")
    expected = {spec["package_id"] for spec in specs}
    require(
        set(path.name for path in packages_dir.iterdir()) == expected,
        "staged input must contain exactly the complete composition",
    )
    packages: dict[str, dict[str, Any]] = {}
    ownership: dict[str, str] = {}
    sizes: dict[str, int] = {}
    for spec in specs:
        package_id = spec["package_id"]
        document = validate_package(packages_dir / package_id, spec, arch, source_commit)
        packages[package_id] = document
        for name in document["files"]:
            require(name not in ownership, f"file ownership conflict: {name}")
            ownership[name] = package_id
            sizes[name] = regular(packages_dir / package_id / "payload" / name).st_size
    for name in ownership:
        for parent in PurePosixPath(name).parents:
            require(str(parent) not in ownership, f"file is another package's directory: {name}")
    for package_id, document in packages.items():
        for dependency in document["dependencies"]:
            target = packages.get(dependency["package_id"])
            require(
                target is not None and target["version"] == dependency["version"],
                f"unsatisfied exact dependency: {package_id}/{dependency}",
            )
    check_budget(list(packages.values()), sizes)
    ordered: list[dict[str, Any]] = []
    emitted: set[str] = set()
    while len(ordered) < len(packages):
        ready = [
            spec["package_id"]
            for spec in specs
            if spec["package_id"] not in emitted
            and all(
                dep["package_id"] in emitted for dep in packages[spec["package_id"]]["dependencies"]
            )
        ]
        require(bool(ready), "cycle in complete release dependencies")
        for package_id in ready:
            ordered.append(packages[package_id])
            emitted.add(package_id)
    return ordered
