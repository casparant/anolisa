#!/usr/bin/env python3
"""Resolve the same product package selection for raw, RPM wrappers and OCI plans."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

try:
    from .recipes import load_recipes, package_specs, require
except ImportError:
    from recipes import load_recipes, package_specs, require


def closure(selected: list[str], specs: list[dict[str, Any]]) -> list[str]:
    """Resolve a stable dependency order and reject missing or cyclic requirements."""
    by_id = {spec["package_id"]: spec for spec in specs}
    result: list[str] = []
    pending: set[str] = set()
    done: set[str] = set()

    def visit(name: str) -> None:
        require(name in by_id, f"unknown product package: {name}")
        require(name not in pending, f"cyclic product dependency: {name}")
        if name in done:
            return
        pending.add(name)
        for dependency in by_id[name]["dependencies"]:
            visit(dependency)
        pending.remove(name)
        done.add(name)
        result.append(name)

    for name in selected:
        visit(name)
    return result


def distribution(source: Path) -> dict[str, Any]:
    """Expand declarative selections without copying any component package data."""
    config = json.loads((source / "distribution/release/linux.json").read_text())
    require(config.get("schema_version") == 1, "unsupported product configuration")
    recipes = load_recipes(source)
    specs = package_specs(source)
    by_id = {spec["package_id"]: spec for spec in specs}
    profiles = {}
    for name, selection in config["profiles"].items():
        selected = list(selection["packages"])
        selected += [
            spec["package_id"]
            for spec in specs
            if spec["kind"] in selection.get("kinds", [])
            and ("components" not in selection or spec["component"] in selection["components"])
        ]
        packages = closure(selected, specs)
        require("anolisa-cli" in packages, f"profile lacks the product installer: {name}")
        scopes = selection["scopes"]
        require(scopes and set(scopes) <= {"user", "system"}, f"invalid profile scopes: {name}")
        profiles[name] = {"packages": packages, "scopes": scopes, "preset": selection["preset"]}
    native = config.get("native_profiles")
    if native:
        require(native["base"] in profiles, "native profile references unknown base")
        base = profiles[native["base"]]["packages"]
        agents = sorted(
            {
                agent
                for recipe in recipes
                for package in recipe["packages"]
                if "linux" in package.get("native_platforms", ["linux"])
                for agent in package.get("agents", [])
                if "linux" in package.get("agent_platforms", {}).get(agent, ["linux"])
            }
        )
        for agent in agents:
            packages = base + [
                package["id"]
                for recipe in recipes
                for package in recipe["packages"]
                if agent in package.get("agents", []) and package.get("kind") != "meta"
            ]
            # Native composition does not install an unused Provider bridge.
            packages = [name for name in packages if by_id[name]["kind"] != "provider"]
            profiles["native-" + agent] = {
                "packages": closure(packages, specs),
                "scopes": native["scopes"],
                "preset": native["preset"],
            }
    services_path = source / config["services"]
    services = json.loads(services_path.read_text())
    require(services.get("schema_version") == 1, "unsupported product service schema")
    service_ids = {service["id"] for service in services["services"]}
    require(len(service_ids) == len(services["services"]), "duplicate product service identity")
    for service in services["services"]:
        require(
            service["package"] in by_id,
            f"unknown service package: {service['id']}/{service['package']}",
        )
    presets = {}
    for name, preset in services["presets"].items():
        selected = [service["id"] for service in preset["services"]]
        require(
            len(selected) == len(set(selected)) and set(selected) <= service_ids,
            f"invalid service preset: {name}",
        )
        presets[name] = {"services": selected}
    for name, profile in profiles.items():
        require(profile["preset"] in presets, f"unknown preset: {name}/{profile['preset']}")
    for scope, name in config["default_profiles"].items():
        require(
            name in profiles and scope in profiles[name]["scopes"],
            f"invalid default profile: {scope}",
        )
    return {
        "schema_version": 1,
        "default_profiles": config["default_profiles"],
        "profiles": profiles,
        "components": {
            recipe["id"]: closure(
                [package["id"] for package in recipe["packages"] if package.get("kind") != "meta"],
                specs,
            )
            for recipe in recipes
        },
        "presets": presets,
    }


def rpm_name(package_id: str) -> str:
    """Preserve ANOLISA's existing native CLI package name."""
    return "anolisa" if package_id == "anolisa-cli" else package_id


def rpm_spec(plan: dict[str, Any], profile: str, version: str, source: Path | None = None) -> str:
    """Generate a file-free wrapper whose dependencies implement the selected profile."""
    require(profile in plan["profiles"], f"unknown profile: {profile}")
    require(
        all(
            character.isascii() and (character.isalnum() or character in ".+_~-")
            for character in version
        )
        and bool(version),
        "unsafe product RPM version",
    )
    version = version.replace("-", ".").replace("+", ".")
    name = (
        "anolisa-packages"
        if profile == "complete"
        else ("anolisa-standard" if profile == "standard-system" else "anolisa-" + profile)
    )
    source = source or Path(__file__).resolve().parents[2]
    native = {
        package["id"]: package.get("rpm_name", rpm_name(package["id"]))
        for recipe in load_recipes(source)
        for package in recipe["packages"]
        if "rpm" in package.get("formats", ["raw", "rpm"])
    }
    requirements = "\n".join(
        "Requires:       " + native[package]
        for package in plan["profiles"][profile]["packages"]
        if package in native
    )
    return f"""Name:           {name}
Version:        {version}
Release:        1%{{?dist}}
Summary:        ANOLISA {profile} product composition
License:        Apache-2.0
BuildArch:      noarch
Requires:       anolisa-policy
{requirements}

%description
Installs the {profile} composition. Service activation remains an explicit
administrator or user deployment action.

%prep
%build
%install

%files
"""


def oci_plan(source: Path, plan: dict[str, Any], profile: str) -> dict[str, Any]:
    """Describe container files and allowed foreground services from the shared policy."""
    require(profile in plan["profiles"], f"unknown profile: {profile}")
    selected = plan["profiles"][profile]
    config = json.loads((source / "distribution/release/services.json").read_text())
    services = [
        service
        for service in config["services"]
        if service["package"] in selected["packages"] and service.get("container_allowed")
    ]
    return {
        "schema_version": 1,
        "profile": profile,
        "packages": selected["packages"],
        "entrypoint": [
            "anolisa",
            "serve",
            "--config",
            "/usr/share/anolisa/distribution/services.json",
            "--preset",
            "container",
        ],
        "init": True,
        "services": services,
    }


def systemd_preset(source: Path, name: str) -> str:
    """Emit only declared product units, preserving host administrator overrides."""
    config = json.loads((source / "distribution/release/services.json").read_text())
    require(name in config["presets"], f"unknown service preset: {name}")
    services = {service["id"]: service for service in config["services"]}
    result = ["# Generated from ANOLISA's shared service policy; explicit start is separate."]
    for selection in config["presets"][name]["services"]:
        unit = services[selection["id"]].get("unit")
        if unit:
            require(
                "/" not in unit and "*" not in unit and unit.endswith(".service"),
                f"invalid product systemd unit: {unit}",
            )
            result.append(("enable " if selection["enable"] else "disable ") + unit)
    return "\n".join(result) + "\n"


def main() -> None:
    """Generate product metadata without invoking a compiler or native package manager."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument(
        "--format", choices=("json", "rpm-spec", "oci-plan", "systemd-preset"), default="json"
    )
    parser.add_argument("--profile", default="complete")
    parser.add_argument("--preset", default="disabled")
    parser.add_argument("--version", default="0.1.0")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    plan = distribution(args.source)
    if args.format == "rpm-spec":
        value = rpm_spec(plan, args.profile, args.version, args.source)
    elif args.format == "systemd-preset":
        value = systemd_preset(args.source, args.preset)
    else:
        value = (
            json.dumps(
                oci_plan(args.source, plan, args.profile) if args.format == "oci-plan" else plan,
                sort_keys=True,
                indent=2,
            )
            + "\n"
        )
    if args.output:
        args.output.write_text(value)
    else:
        print(value, end="")


if __name__ == "__main__":
    main()
