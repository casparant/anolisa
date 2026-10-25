#!/usr/bin/env python3
"""Carry central release recipes with a standalone component source archive."""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path


def snapshot(source: Path, destination: Path, component: str) -> None:
    """Copy the recipe and shared tools without following arbitrary source links."""
    recipe = source / "distribution/components" / component
    if not recipe.is_dir():
        recipe = source / "distribution/providers" / component
    if not recipe.is_dir() or recipe.is_symlink():
        raise ValueError(f"missing central component recipe: {component}")
    inputs = [recipe, source / "distribution/packaging"]
    inputs += [
        source / "distribution/anolisa/packaging/release" / name
        for name in ("stage.py", "release_common.py")
    ]
    if component == "anolisa-cli":
        inputs += [source / "distribution/release"]
        inputs += sorted((source / "distribution/components").iterdir())
        inputs += sorted((source / "distribution/providers").iterdir())
        inputs += [source / "distribution/crates/local-package"]
    if component == "aw-core" or component.startswith("aw-provider-"):
        inputs += [
            source / "distribution/crates/local-package",
            source / "distribution/tooling/aw-preview",
        ]
    if component.startswith("aw-provider-"):
        inputs += [
            source / "providers/tokenless",
            source / "providers/agent-sec-core",
            source / "src/aw",
        ]
        inputs += [
            source / "src/agent-sec-core/packaging/systemd" / name
            for name in ("agent-sec-core-v2.service.in", "skillsec.json")
        ]
        target = destination / "LICENSE"
        if not target.exists():
            shutil.copyfile(source / "LICENSE", target)
    recipe_roots = [root for root in inputs if (root / "recipe.json").is_file()]
    flattened_roots: dict[Path, Path] = {}
    for recipe_root in recipe_roots:
        definition = json.loads((recipe_root / "recipe.json").read_text())
        component_root = (source / definition["source_dir"]).resolve()
        if not component_root.is_relative_to(source.resolve()):
            raise ValueError(f"component source escapes the source snapshot: {recipe_root}")
        for field in ("rpm_spec", "raw_script"):
            if field not in definition:
                continue
            owned = source / definition[field]
            resolved = owned.resolve(strict=True)
            # CLI, providers and cosh retain canonical inputs in their recipe tree.
            # That tree is already an explicit snapshot input; no new source is trusted.
            recipe_owned = owned.is_relative_to(recipe_root) and resolved.is_relative_to(
                recipe_root.resolve()
            )
            if not resolved.is_relative_to(component_root) and not recipe_owned:
                raise ValueError(f"component build input escapes its source directory: {owned}")
            inputs.append(resolved.parent if field == "raw_script" else resolved)
            if recipe_root == recipe and resolved.is_relative_to(component_root):
                flattened_roots[inputs[-1]] = component_root

    def trusted_file(path: Path) -> Path:
        if not path.is_symlink():
            return path
        resolved = path.resolve(strict=True)
        if path.name == "LICENSE" and resolved == (source / "LICENSE").resolve():
            return resolved
        # Distribution compatibility links may reference only that recipe's
        # component-owned sources. Other source symlinks remain rejected.
        for recipe_root in recipe_roots:
            if path.is_relative_to(recipe_root):
                definition = json.loads((recipe_root / "recipe.json").read_text())
                component_root = (source / definition["source_dir"]).resolve()
                if resolved.is_relative_to(component_root) and resolved.is_file():
                    return resolved
        raise ValueError(f"recipe snapshot is not an ordinary owned file: {path}")

    for root in inputs:
        for path in ([root] if root.is_file() else sorted(root.rglob("*"))):
            if (
                path.is_dir()
                or any(
                    part in {"__pycache__", "target", "node_modules", ".git"}
                    for part in path.relative_to(root).parts
                )
                or path.suffix == ".pyc"
            ):
                continue
            resolved_path = trusted_file(path)
            if resolved_path.is_symlink() or not resolved_path.is_file():
                raise ValueError(f"recipe snapshot is not an ordinary file: {path}")
            targets = [destination / path.relative_to(source)]
            if root in flattened_roots:
                # Native component archives place their source tree at the archive root.
                targets.append(destination / path.relative_to(flattened_roots[root]))
            for target in targets:
                target.parent.mkdir(parents=True, exist_ok=True)
                if target.exists():
                    if target.read_bytes() != resolved_path.read_bytes():
                        raise ValueError(f"recipe snapshot conflicts with source: {target}")
                    continue
                shutil.copyfile(resolved_path, target, follow_symlinks=False)
                target.chmod(0o755 if resolved_path.stat().st_mode & 0o111 else 0o644)
    if component == "anolisa-cli":
        manifest = destination / "Cargo.toml"
        if manifest.is_file():
            # Standalone sources flatten the CLI workspace into the archive root.
            # Keep its sibling shared library inside that same source archive.
            manifest.write_text(
                manifest.read_text().replace(
                    'local-package = { path = "../crates/local-package" }',
                    'local-package = { path = "distribution/crates/local-package" }',
                )
            )
    document = json.loads((recipe / "recipe.json").read_text())
    source_component = source / document["source_dir"]
    # Resolve only the explicit repository compatibility links for this component.
    for alias in source_component.rglob("*"):
        if not alias.is_symlink():
            continue
        resolved = alias.resolve(strict=True)
        if not resolved.is_relative_to(recipe.resolve()):
            continue
        target = destination / alias.relative_to(source_component)
        if target.is_symlink():
            target.unlink()
        if resolved.is_dir():
            shutil.copytree(
                resolved,
                target,
                dirs_exist_ok=True,
                symlinks=False,
                ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "node_modules", "target"),
            )
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists() and target.read_bytes() != resolved.read_bytes():
                raise ValueError(f"recipe alias conflicts with source: {target}")
            shutil.copyfile(resolved, target)
            target.chmod(0o755 if resolved.stat().st_mode & 0o111 else 0o644)


def main() -> None:
    """Prepare an existing source staging tree before source/SRPM archiving."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--component", required=True)
    args = parser.parse_args()
    snapshot(args.source, args.destination, args.component)


if __name__ == "__main__":
    main()
