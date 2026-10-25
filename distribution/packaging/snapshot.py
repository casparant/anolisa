#!/usr/bin/env python3
"""Carry optional distribution integration in standalone component sources."""

from __future__ import annotations

import argparse
import json
import shutil
import tomllib
from pathlib import Path

try:
    from .recipes import recipe_path, recipe_paths, safe_path
except ImportError:
    from recipes import recipe_path, recipe_paths, safe_path


def snapshot(source: Path, destination: Path, component: str) -> None:
    """Copy the recipe and shared tools without following arbitrary source links."""
    source = source.resolve()
    definition_path = recipe_path(source, component)
    if definition_path is None:
        raise ValueError(f"missing component recipe: {component}")
    recipe = definition_path.parent
    selected = json.loads((recipe / "recipe.json").read_text())
    selected_source = source / selected["source_dir"]
    inputs = [recipe, source / "distribution/packaging"]
    inputs += [
        source / "distribution/anolisa/packaging/release" / name
        for name in ("stage.py", "release_common.py")
    ]
    if component == "anolisa-cli":
        inputs += [source / "distribution/release"]
        inputs += [path.parent for path in recipe_paths(source) if path.parent != recipe]
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
    flattened_roots: dict[Path, Path] = {recipe: selected_source}
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
            if not resolved.is_relative_to(component_root):
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
    for recipe_root in recipe_roots:
        template = recipe_root / "component.toml.in"
        if not template.is_file():
            continue
        definition = json.loads((recipe_root / "recipe.json").read_text())
        rendered = template.read_text()
        if "@VERSION@" in rendered:
            build = definition["build"]
            version_file = source / safe_path(build["version_file"])
            component_root = (source / safe_path(definition["source_dir"])).resolve()
            if not version_file.resolve().is_relative_to(component_root):
                raise ValueError(f"component version escapes its source directory: {version_file}")
            value = tomllib.loads(version_file.read_text())
            for key in build["version_key"].split("."):
                value = value[key]
            rendered = rendered.replace("@VERSION@", str(value))
        targets = [destination / recipe_root.relative_to(source) / "component.toml"]
        if recipe_root == recipe:
            targets.append(destination / ".anolisa/component.toml")
        for target in targets:
            if target.is_symlink():
                target.unlink()
            if target.exists() and target.read_text() not in {template.read_text(), rendered}:
                raise ValueError(f"generated contract conflicts with source: {target}")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(rendered)
            target.chmod(0o644)
    if component in {"anolisa-cli", "aw-core"}:
        manifest = destination / "Cargo.toml"
        if manifest.is_file():
            # Flattened sources retain sibling workspaces inside the archive without
            # allowing Cargo to absorb them as members of the parent workspace.
            prefix = (
                "../crates/local-package"
                if component == "anolisa-cli"
                else "../../distribution/crates/local-package"
            )
            contents = manifest.read_text().replace(
                f'path = "{prefix}', 'path = "distribution/crates/local-package'
            )
            excluded = 'exclude = ["distribution/crates/local-package", "distribution/tooling/aw-preview"]\n'
            if excluded not in contents:
                contents = contents.replace("[workspace]\n", "[workspace]\n" + excluded, 1)
            manifest.write_text(contents)
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
