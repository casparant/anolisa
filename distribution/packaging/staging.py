"""Normalize historical raw layouts before assigning immutable subpackage owners."""

from __future__ import annotations

import shutil
from pathlib import Path
from typing import Any

try:
    from .recipes import partition, require
except ImportError:
    from recipes import partition, require


def canonical_path(recipe: dict[str, Any], name: str) -> str:
    """Map only known old component locations; product and AW resources stay separate."""
    component, namespace = recipe["id"], recipe["resource_namespace"]
    if (
        name == ".anolisa/component.toml"
        or name == f"share/anolisa/components/{component}/source-component.toml"
    ):
        return f"share/{namespace}/metadata/component.toml"
    replacements = [
        (f"share/anolisa/runtime/{component}/plugins/", f"share/{namespace}/adapters/"),
        (f"share/anolisa/adapters/{component}/", f"share/{namespace}/adapters/"),
        (f"share/anolisa/runtime/skills/", f"share/{namespace}/skills/"),
        (f"share/anolisa/skills/", f"share/{namespace}/skills/"),
        (f"share/anolisa/extensions/{component}/", f"share/{namespace}/adapters/cosh/"),
        (f"share/anolisa/{component}/", f"share/{namespace}/templates/"),
    ]
    if component == "sec-core":
        replacements += [
            ("share/anolisa/extensions/agent-sec-core/", f"share/{namespace}/adapters/cosh/"),
            ("libexec/anolisa/sec-core/", "libexec/agent-sec-core/"),
        ]
    if component == "agent-memory":
        replacements += [("share/anolisa/mcp-servers/", "share/agent-memory/mcp-servers/")]
    if component == "cosh-ng":
        replacements = [
            (f"share/anolisa/{component}/", f"share/{namespace}/templates/")
        ] + replacements
    if component == "os-skills":
        replacements = [("share/anolisa/skills/", "share/os-skills/skills/")] + replacements
    if component in {"aw-core", "anolisa-cli"} or component.startswith("aw-provider-"):
        return name
    for before, after in replacements:
        if name.startswith(before):
            return after + name[len(before) :]
    return name


def normalize(recipe: dict[str, Any], source: Path, destination: Path) -> None:
    """Move readonly resources once and detect historical duplicate copies by content."""
    destination.mkdir(parents=True, exist_ok=False)
    for path in sorted(source.rglob("*")):
        if path.is_dir():
            continue
        require(path.is_file() and not path.is_symlink(), f"unsupported staged file: {path}")
        name = canonical_path(recipe, path.relative_to(source).as_posix())
        target = destination / name
        if target.exists():
            require(
                target.read_bytes() == path.read_bytes()
                and target.stat().st_mode & 0o777 == path.stat().st_mode & 0o777,
                f"conflicting duplicate component resource: {name}",
            )
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
        target.chmod(0o755 if path.stat().st_mode & 0o111 else 0o644)


def split(
    recipe: dict[str, Any], source: Path, destination: Path, license_file: Path | None = None
) -> list[str]:
    """Copy the canonical input into uniquely owned nonempty package payloads."""
    inventory = {
        path.relative_to(source).as_posix(): path for path in source.rglob("*") if path.is_file()
    }
    selected = partition(recipe, list(inventory))
    for package_id, owned in selected.items():
        payload = destination / package_id / "payload"
        require(not payload.exists(), f"refusing existing staged package: {package_id}")
        for name, origin in owned.items():
            target = payload / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(inventory[origin], target)
            target.chmod(0o755 if inventory[origin].stat().st_mode & 0o111 else 0o644)
        if license_file is not None:
            target = payload / "share/doc" / package_id / "LICENSE"
            if not target.exists():
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(license_file, target)
                target.chmod(0o644)
    return list(selected)
