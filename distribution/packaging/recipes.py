"""Resolve component-owned package recipes for every distribution format."""

from __future__ import annotations

import json
import re
from pathlib import Path, PurePosixPath
from typing import Any


class RecipeError(ValueError):
    """A recipe is ambiguous or cannot own the requested payload."""


def require(condition: bool, message: str) -> None:
    """Reject a recipe without silently choosing a package owner."""
    if not condition:
        raise RecipeError(message)


def safe_path(value: Any) -> str:
    """Restrict recipe paths to normal relative filesystem paths."""
    require(isinstance(value, str) and bool(value), "empty recipe path")
    value = value.rstrip("/")
    parts = value.split("/")
    require(
        not value.startswith("/")
        and "\\" not in value
        and all(part not in {"", ".", ".."} for part in parts)
        and not any(ord(char) < 32 for char in value),
        f"unsafe recipe path: {value!r}",
    )
    return value


def load_recipes(source: Path, allow_external: bool = False) -> list[dict[str, Any]]:
    """Load the central recipes, checking identities and exact local dependencies."""
    result = []
    seen: set[str] = set()
    package_ids: set[str] = set()
    paths = list((source / "distribution/components").glob("*/recipe.json"))
    paths += list((source / "distribution/providers").glob("*/recipe.json"))
    for path in sorted(paths):
        document = json.loads(path.read_text(encoding="utf-8"))
        require(document.get("schema_version") == 1, f"unsupported recipe: {path}")
        identity = document.get("id")
        require(
            isinstance(identity, str)
            and re.fullmatch(r"[a-z0-9][a-z0-9._-]*", identity) is not None
            and identity not in seen,
            f"invalid or duplicate component identity: {path}",
        )
        seen.add(identity)
        require(isinstance(document.get("build"), dict), f"missing build recipe: {identity}")
        require(bool(document.get("packages")), f"no packages in recipe: {identity}")
        for package in document["packages"]:
            name = package["id"]
            require(name not in package_ids, f"duplicate package recipe: {name}")
            require(
                re.fullmatch(r"[a-z0-9][a-z0-9._-]*", name) is not None, f"invalid package: {name}"
            )
            package_ids.add(name)
            for mapping in package.get("sources", []):
                safe_path(mapping["from"])
                safe_path(mapping.get("to", mapping["from"]))
        result.append(document)
    require(bool(result), "no central component recipes")
    for recipe in result:
        for package in recipe["packages"]:
            for dependency in package.get("requires", []):
                require(
                    allow_external or dependency in package_ids,
                    f"unknown package dependency: {package['id']}/{dependency}",
                )
    return result


def build_specs(source: Path) -> list[dict[str, Any]]:
    """Return unique compilation inputs independently of the number of subpackages."""
    return [dict(recipe["build"]) for recipe in load_recipes(source)]


def matched(name: str, prefix: str) -> bool:
    """Match a source file or a directory boundary, never a string prefix alone."""
    prefix = safe_path(prefix)
    return name == prefix or name.startswith(prefix + "/")


def mapping_target(name: str, mapping: dict[str, Any]) -> str:
    """Preserve the directory suffix under a declared destination root."""
    origin = safe_path(mapping["from"])
    target = safe_path(mapping.get("to", origin))
    require(matched(name, origin), f"path {name} does not match recipe source {origin}")
    return target + name[len(origin) :]


def partition(recipe: dict[str, Any], paths: list[str]) -> dict[str, dict[str, str]]:
    """Assign every staged file once, rejecting both missing and overlapping rules."""
    result = {package["id"]: {} for package in recipe["packages"] if package.get("kind") != "meta"}
    for name in sorted(paths):
        safe_path(name)
        candidates = []
        for package in recipe["packages"]:
            if package.get("kind") == "meta":
                continue
            for mapping in package.get("sources", []):
                if matched(name, mapping["from"]) and not any(
                    matched(name, exclude) for exclude in mapping.get("exclude", [])
                ):
                    candidates.append((package["id"], mapping_target(name, mapping)))
        require(len(candidates) == 1, f"{recipe['id']}/{name} has {len(candidates)} package owners")
        owner, destination = candidates[0]
        require(destination not in result[owner], f"duplicate mapped path: {owner}/{destination}")
        result[owner][destination] = name
    for package in recipe["packages"]:
        if package.get("kind") == "meta":
            continue
        require(bool(result[package["id"]]), f"empty payload package: {package['id']}")
        for mapping in package.get("sources", []):
            require(
                mapping.get("optional", False)
                or any(matched(name, mapping["from"]) for name in result[package["id"]].values()),
                f"missing required source: {package['id']}/{mapping['from']}",
            )
    owners = {}
    for owner, selected in result.items():
        for name in selected:
            require(name not in owners, f"duplicate destination: {name}")
            owners[name] = owner
    for name in owners:
        require(
            not any(str(parent) in owners for parent in PurePosixPath(name).parents),
            f"file is a package directory: {name}",
        )
    return result


def package_specs(source: Path, allow_external: bool = False) -> list[dict[str, Any]]:
    """Derive local AW descriptions and public entries from the package recipes."""
    from .staging import canonical_path

    result = []
    for recipe in load_recipes(source, allow_external=allow_external):
        for package in recipe["packages"]:
            if package.get("kind") == "meta":
                continue
            mappings = package.get("sources", [])
            entries = dict(package.get("entrypoints", {}))
            for command, name in recipe["build"].get("entrypoints", {}).items():
                if "entrypoints" in package:
                    continue
                for mapping in mappings:
                    if matched(name, mapping["from"]) and not any(
                        matched(name, exclude) for exclude in mapping.get("exclude", [])
                    ):
                        entries[command] = mapping_target(name, mapping)
            required = [
                safe_path(mapping.get("to", mapping["from"]))
                + ("/" if mapping["from"].endswith("/") else "")
                for mapping in mappings
                if not mapping.get("optional", False)
            ]
            for legacy_name in recipe["build"].get("required", []):
                name = canonical_path(recipe, legacy_name)
                for mapping in mappings:
                    if matched(name, mapping["from"]) and not any(
                        matched(name, exclude) for exclude in mapping.get("exclude", [])
                    ):
                        target = mapping_target(name, mapping)
                        if target not in required:
                            required.append(target)
            result.append(
                {
                    "package_id": package["id"],
                    "component": recipe["id"],
                    "kind": package["kind"],
                    "dependencies": package.get("requires", []),
                    "entrypoints": entries,
                    "required": required,
                    **({"provider": True} if package["kind"] == "provider" else {}),
                }
            )
    return result
