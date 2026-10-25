#!/usr/bin/env python3
"""Create independently distributable local subpackages from a component raw input."""

from __future__ import annotations

import argparse
import os
import sys
import tempfile
from pathlib import Path

SOURCE = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(SOURCE))
sys.path.insert(0, str(SOURCE / "distribution/anolisa/packaging/release"))
from distribution.packaging.recipes import load_recipes, package_specs, require
from release_common import COMMIT, files, version
from stage import cargo_package, raw_package, split_component, unpack_raw, write_description


def stage_component(
    source: Path,
    component: str,
    raw_input: Path | None,
    binary: Path | None,
    destination: Path,
    component_version: str,
    source_commit: str,
    arch: str,
    dependencies: dict[str, str],
) -> None:
    """Publish complete package directories while keeping external dependencies explicit."""
    require(COMMIT.fullmatch(source_commit) is not None, "source-commit must be 40 lowercase hex")
    version(component_version)
    require(arch in {"x86_64", "aarch64"}, "unsupported architecture")
    recipes = load_recipes(source, allow_external=True)
    selected = [recipe for recipe in recipes if recipe["id"] == component]
    require(len(selected) == 1, f"unknown component recipe: {component}")
    recipe = selected[0]
    require((raw_input is None) != (binary is None), "provide exactly one raw input or binary")
    require(not destination.exists(), f"refusing existing package output: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".component-packages-", dir=destination.parent) as work:
        root = Path(work)
        target = root / "component"
        if raw_input is not None:
            if raw_input.is_dir():
                raw = raw_input
                files(raw)
            else:
                raw = root / "raw"
                unpack_raw(raw_input, raw)
            raw_package(raw, target, recipe["build"], source)
        else:
            require(binary is not None, "missing prebuilt binary")
            cargo_package(binary, target, recipe["build"], source)
        packages = root / "packages"
        emitted = split_component(recipe, target / "payload", packages, root / "canonical", source)
        versions = dict(dependencies)
        versions.update({name: component_version for name in emitted})
        for spec in package_specs(source, allow_external=True):
            if spec["package_id"] not in emitted:
                continue
            for dependency in spec["dependencies"]:
                require(
                    dependency in versions, f"provide --dependency-version {dependency}=VERSION"
                )
                version(versions[dependency])
            write_description(packages / spec["package_id"], spec, versions, arch, source_commit)
        os.rename(packages, destination)


def main() -> None:
    """Use already-built inputs; neither source builds nor package scripts are executed."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("--component", required=True)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--raw-input", type=Path)
    group.add_argument("--binary", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--arch", choices=("x86_64", "aarch64"), required=True)
    parser.add_argument("--dependency-version", action="append", default=[])
    args = parser.parse_args()
    dependencies = {}
    for value in args.dependency_version:
        name, separator, exact = value.partition("=")
        require(
            bool(separator) and name not in dependencies, "dependency must be a unique ID=VERSION"
        )
        dependencies[name] = version(exact)
    stage_component(
        args.source,
        args.component,
        args.raw_input,
        args.binary,
        args.output,
        args.version,
        args.source_commit,
        args.arch,
        dependencies,
    )


if __name__ == "__main__":
    main()
