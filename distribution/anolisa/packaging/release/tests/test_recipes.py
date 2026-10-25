"""Check package ownership, shared product policy and standalone source portability."""

from __future__ import annotations

import json
import os
import shutil
import sys
import subprocess
import tarfile
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "distribution/anolisa/packaging/release"))

from distribution.packaging.archive import archive
from distribution.packaging.product import distribution, oci_plan, rpm_spec, systemd_preset
from distribution.packaging.recipes import (
    load_recipes,
    package_specs,
    partition,
    recipe_path,
    recipe_paths,
)
from distribution.packaging.snapshot import snapshot
from distribution.packaging.staging import normalize
from release_common import elf


class RecipeContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(
            dir="/private/tmp" if Path("/private/tmp").is_dir() else None
        )
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def rendered_contract(self, component: str, filename: str = "component.toml.in") -> Path:
        recipe = json.loads(recipe_path(ROOT, component).read_text())
        build = recipe["build"]
        version = tomllib.loads((ROOT / build["version_file"]).read_text())
        for key in build["version_key"].split("."):
            version = version[key]
        template = ROOT / recipe["source_dir"] / ".anolisa" / filename
        document = template.read_text().replace("@VERSION@", version)
        self.assertEqual(tomllib.loads(document)["component"]["version"], version)
        target = self.root / f"{component}-{filename.removesuffix('.in')}"
        target.write_text(document)
        return target

    def test_prebuilt_actions_generate_ordinary_contracts_in_fresh_component_sources(self) -> None:
        for component in ("tokenless", "agent-memory"):
            with self.subTest(component=component):
                origin = ROOT / "src" / component
                worktree = self.root / component / "worktree"
                target = worktree / "src" / component
                target.mkdir(parents=True)
                for name in ("Makefile", "Cargo.toml"):
                    shutil.copyfile(origin / name, target / name)
                if component == "tokenless":
                    (target / "packaging/raw").mkdir(parents=True)
                    for name in ("wheel-version.py", "verify-release.py"):
                        shutil.copyfile(
                            origin / "packaging/raw" / name, target / "packaging/raw" / name
                        )
                for template in origin.rglob("*.in"):
                    relative = template.relative_to(origin)
                    if {"target", "node_modules", ".git"}.intersection(relative.parts):
                        continue
                    destination = target / relative
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(template, destination)
                canonical = target / ".anolisa/component.toml.in"
                before = canonical.read_bytes()
                generated = target / ".anolisa/component.toml"
                self.assertFalse(generated.exists())
                build = (ROOT / f".github/actions/build-{component}-prebuilt/build.sh").read_text()
                start = build.index('COMPONENT_ROOT="$FIXED_WORKTREE/src/')
                marker = (
                    'SOURCE_VERSION="$(' if component == "tokenless" else 'COMPONENT_VERSION="$('
                )
                preparation = build[start : build.index(marker, start)]
                result = subprocess.run(
                    ["bash", "-euo", "pipefail", "-c", preparation],
                    env={**os.environ, "FIXED_WORKTREE": str(worktree)},
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(generated.is_file())
                self.assertFalse(generated.is_symlink())
                source = tomllib.loads((origin / "Cargo.toml").read_text())
                expected = (
                    source["workspace"]["package"]["version"]
                    if component == "tokenless"
                    else source["package"]["version"]
                )
                self.assertEqual(
                    tomllib.loads(generated.read_text())["component"]["version"], expected
                )
                self.assertEqual(canonical.read_bytes(), before)

    def test_recipe_discovery_uses_only_component_integration_directories(self) -> None:
        source = self.root / "discovery"
        selected = {
            "backend": "src/backend/.anolisa/recipe.json",
            "provider": "providers/backend/.anolisa/recipe.json",
            "anolisa-cli": "distribution/anolisa/.anolisa/recipe.json",
        }
        ignored = (
            "distribution/components/legacy/recipe.json",
            "distribution/providers/legacy/recipe.json",
            "src/backend/unrelated/.anolisa/recipe.json",
        )
        for identity, relative in selected.items():
            path = source / relative
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps({"id": identity}))
        for relative in ignored:
            path = source / relative
            path.parent.mkdir(parents=True)
            path.write_text("unrelated metadata must not be parsed")
        self.assertEqual(set(recipe_paths(source)), {source / value for value in selected.values()})
        for identity, relative in selected.items():
            self.assertEqual(recipe_path(source, identity), source / relative)

    def test_recipe_discovery_rejects_duplicate_component_identities(self) -> None:
        source = self.root / "duplicates"
        for component in ("one", "two"):
            path = source / "src" / component / ".anolisa/recipe.json"
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps({"id": "duplicate"}))
        with self.assertRaisesRegex(ValueError, "duplicate component recipe"):
            recipe_path(source, "duplicate")

    def test_recipe_discovery_rejects_external_metadata_links(self) -> None:
        source = self.root / "linked-source"
        component = source / "src/backend"
        component.mkdir(parents=True)
        external = self.root / "external-metadata"
        external.mkdir()
        (external / "recipe.json").write_text(json.dumps({"id": "backend"}))
        (component / ".anolisa").symlink_to(external, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "ordinary owned file"):
            recipe_paths(source)

    def test_optional_recipe_cli_keeps_legacy_packages_without_integration_supported(self) -> None:
        arguments = [
            sys.executable,
            str(ROOT / "distribution/packaging/recipes.py"),
            "--source",
            str(self.root),
            "--component",
            "legacy-sandbox",
        ]
        optional = subprocess.run(arguments + ["--optional"], capture_output=True, text=True)
        self.assertEqual(optional.returncode, 0, optional.stderr)
        self.assertEqual(optional.stdout, "")
        strict = subprocess.run(arguments, capture_output=True, text=True)
        self.assertNotEqual(strict.returncode, 0)
        self.assertIn("missing or duplicate component recipe", strict.stderr)

    def test_tokenless_distribution_contract_covers_native_adapters(self) -> None:
        template = ROOT / "src/tokenless/.anolisa/component.toml.in"
        contract = tomllib.loads(template.read_text())
        sources = {
            entry["source"].rstrip("/")
            for entry in contract["component"]["layout"]["files"] + contract["adapters"]
        }
        self.assertTrue(
            {
                "bin/tokenless",
                "share/tokenless/adapters/cosh",
                "share/tokenless/adapters/manifest.json",
            }
            <= sources
        )
        adapters = {entry["framework"]: entry for entry in contract["adapters"]}
        targets = set(
            json.loads((ROOT / "src/tokenless/adapters/tokenless/manifest.json.in").read_text())[
                "targets"
            ]
        )
        self.assertIn("qwenwork", targets)
        self.assertEqual(set(adapters), targets)
        self.assertEqual(
            {entry["when"] for entry in adapters["opencode"]["notices"]},
            {"post_enable", "post_disable"},
        )
        for document in (
            contract,
            tomllib.loads(self.rendered_contract("tokenless").read_text()),
        ):
            matches = [entry for entry in document["adapters"] if entry["framework"] == "qwenwork"]
            self.assertEqual(len(matches), 1)
            for key, value in {
                "adapter_type": "plugin",
                "plugin_id": "tokenless",
                "source": "share/tokenless/adapters/qwenwork",
                "dest": "{sharedir}/tokenless/adapters/qwenwork/",
            }.items():
                self.assertEqual(matches[0][key], value)
            self.assertEqual(matches[0]["bundle"]["entry"], ".qoder-plugin/plugin.json")

    def test_distribution_contracts_match_real_component_raw_payloads(self) -> None:
        for component, case in (
            (
                "ws-ckpt",
                "WsCkptPackagingTests.test_payload_satisfies_contract_and_runtime_discovery",
            ),
            ("skillfs", "SkillfsPackagingTests.test_payload_and_permissions_on_both_architectures"),
        ):
            contract = self.rendered_contract(component)
            result = subprocess.run(
                [sys.executable, str(ROOT / f"src/{component}/tests/test_raw_packaging.py"), case],
                env={**os.environ, "PACKAGING_TEST_CONTRACT": str(contract)},
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_shell_component_contracts_match_raw_payloads(self) -> None:
        for script, environment in (
            (
                "src/agentsight/scripts/test-package-raw.sh",
                {
                    "PACKAGING_TEST_CONTRACT_LINUX": str(self.rendered_contract("agentsight")),
                    "PACKAGING_TEST_CONTRACT_MACOS": str(
                        self.rendered_contract("agentsight", "component.macos.toml.in")
                    ),
                },
            ),
            (
                "src/agent-sec-core/tests/packaging/test-package-raw.sh",
                {
                    "PACKAGING_TEST_CONTRACT": str(self.rendered_contract("sec-core")),
                },
            ),
        ):
            with self.subTest(script=script):
                result = subprocess.run(
                    ["bash", str(ROOT / script)],
                    env={**os.environ, **environment},
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_every_component_partitions_all_required_groups_without_overlap(self) -> None:
        recipes = load_recipes(ROOT)
        self.assertEqual(len(recipes), 13)
        self.assertGreater(len(package_specs(ROOT)), 20)
        for recipe in recipes:
            names = set()
            for package in recipe["packages"]:
                for entry in package.get("sources", []):
                    if entry.get("optional"):
                        continue
                    origin = entry["from"]
                    names.add(
                        origin + "fixture-" + package["id"] if origin.endswith("/") else origin
                    )
            with self.subTest(component=recipe["id"]):
                owned = partition(recipe, list(names))
                self.assertEqual(sum(len(paths) for paths in owned.values()), len(names))
                with self.assertRaisesRegex(ValueError, "package owners"):
                    partition(recipe, list(names) + ["share/unknown/unowned"])

    def test_overlapping_source_rules_fail_even_for_identical_files(self) -> None:
        recipe = {
            "id": "fixture",
            "packages": [
                {"id": "one", "sources": [{"from": "share/data/"}]},
                {"id": "two", "sources": [{"from": "share/data/file"}]},
            ],
        }
        with self.assertRaisesRegex(ValueError, "2 package owners"):
            partition(recipe, ["share/data/file"])

    def test_historical_skill_copies_normalize_once_and_conflicts_fail(self) -> None:
        source = self.root / "legacy"
        for prefix in ("share/anolisa/runtime/skills", "share/anolisa/skills"):
            path = source / prefix / "ws-ckpt/SKILL.md"
            path.parent.mkdir(parents=True)
            path.write_bytes(b"same skill")
        recipe = {"id": "ws-ckpt", "resource_namespace": "ws-ckpt"}
        target = self.root / "canonical"
        normalize(recipe, source, target)
        self.assertEqual(len(list(target.rglob("SKILL.md"))), 1)
        (source / "share/anolisa/skills/ws-ckpt/SKILL.md").write_bytes(b"divergent skill")
        with self.assertRaisesRegex(ValueError, "conflicting duplicate"):
            normalize(recipe, source, self.root / "conflict")

    def test_product_profiles_avoid_native_plugins_and_python_by_default(self) -> None:
        plan = distribution(ROOT)
        system = set(plan["profiles"]["standard-system"]["packages"])
        minimal = set(plan["profiles"]["minimal"]["packages"])
        native = set(plan["profiles"]["native-openclaw"]["packages"])
        self.assertEqual(minimal, {"aw-core", "anolisa-cli"})
        self.assertEqual(plan["default_profiles"], {"user": "complete", "system": "complete"})
        complete = set(plan["profiles"]["complete"]["packages"])
        self.assertEqual(complete, {spec["package_id"] for spec in package_specs(ROOT)})
        self.assertIn("cosh-ng-core", complete)
        self.assertIn("os-skills", complete)
        self.assertIn("agent-sec-python-runtime", complete)
        self.assertIn("Name:           anolisa-packages", rpm_spec(plan, "complete", "1.0.0"))
        self.assertTrue(
            {"tokenless-core", "aw-provider-tokenless", "agent-sec-cli", "aw-provider-sec-core"}
            <= system
        )
        tokenless_plugins = {
            spec["package_id"]
            for spec in package_specs(ROOT)
            if spec["component"] == "tokenless" and spec["kind"] == "plugin"
        }
        self.assertTrue(tokenless_plugins.isdisjoint(system))
        self.assertNotIn("agent-sec-python-runtime", system)
        self.assertIn("agent-sec-python-runtime", native)
        self.assertNotIn("aw-provider-tokenless", native)
        self.assertTrue(tokenless_plugins.intersection(native))
        spec = rpm_spec(plan, "standard-system", "1.0.0-preview.1")
        self.assertIn("Requires:       tokenless-core", spec)
        self.assertIn("Requires:       anolisa-policy", spec)
        self.assertNotIn("Requires:       agent-sec-core\n", spec)
        self.assertIn("Version:        1.0.0.preview.1", spec)
        native_spec = rpm_spec(plan, "native-openclaw", "1.0.0")
        self.assertNotIn("Requires:       agent-sec-python-runtime", native_spec)
        self.assertNotIn("native-qwenwork", plan["profiles"])
        preset = systemd_preset(ROOT, "disabled")
        self.assertIn("disable ws-ckpt.service", preset)
        self.assertNotIn("*", preset)
        self.assertNotIn(
            "start", "\n".join(line for line in preset.splitlines() if not line.startswith("#"))
        )
        container = oci_plan(ROOT, plan, "standard-user")
        self.assertEqual([item["id"] for item in container["services"]], ["aw"])

    def test_portable_archive_is_reproducible_and_rejects_symlinks(self) -> None:
        source = self.root / "stage"
        source.mkdir()
        (source / "file").write_bytes(b"fixture")
        first, second = self.root / "first.tgz", self.root / "second.tgz"
        archive(source, first, 1234)
        os.utime(source / "file", (1, 1))
        archive(source, second, 1234)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        with tarfile.open(first) as stream:
            self.assertTrue(
                all(member.uid == member.gid == 0 and member.mtime == 1234 for member in stream)
            )
        (source / "link").symlink_to("file")
        with self.assertRaises(ValueError):
            archive(source, self.root / "unsafe.tgz", 1234)
        self.assertFalse((self.root / "unsafe.tgz").exists())
        (source / "link").unlink()
        with self.assertRaises(FileExistsError):
            archive(source, first, 1234)

    def test_component_source_snapshot_carries_local_distribution_metadata(self) -> None:
        target = self.root / "ws-source"
        target.mkdir()
        snapshot(ROOT, target, "ws-ckpt")
        self.assertTrue((target / ".anolisa/component.toml").is_file())
        self.assertFalse((target / ".anolisa/component.toml").is_symlink())
        self.assertTrue((target / "packaging/raw/package.sh").is_file())
        self.assertTrue((target / "distribution/packaging/archive.py").is_file())

    def snapshot_fixture(self, rpm_spec: str, raw_script: str | None = None) -> tuple[Path, Path]:
        source = self.root / "source-fixture"
        recipe = source / "src/fixture/.anolisa"
        recipe.mkdir(parents=True)
        document = {"id": "fixture", "source_dir": "src/fixture", "rpm_spec": rpm_spec}
        if raw_script is not None:
            document["raw_script"] = raw_script
        (recipe / "recipe.json").write_text(json.dumps(document))
        return source, recipe

    def test_snapshot_accepts_canonical_recipe_owned_spec(self) -> None:
        source, recipe = self.snapshot_fixture("src/fixture/.anolisa/rpm/fixture.spec.in")
        (recipe / "rpm").mkdir()
        (recipe / "rpm/fixture.spec.in").write_text("fixture spec\n")
        destination = self.root / "recipe-owned-snapshot"
        snapshot(source, destination, "fixture")
        for prefix in ("", "src/fixture/"):
            self.assertEqual(
                (destination / (prefix + ".anolisa/rpm/fixture.spec.in")).read_text(),
                "fixture spec\n",
            )

    def test_fresh_source_snapshot_renders_contract_without_generated_source_file(self) -> None:
        source, recipe = self.snapshot_fixture("src/fixture/.anolisa/rpm/fixture.spec.in")
        (recipe / "rpm").mkdir()
        (recipe / "rpm/fixture.spec.in").write_text("fixture spec\n")
        (source / "src/fixture/Cargo.toml").write_text('[package]\nversion = "1.2.3"\n')
        definition = json.loads((recipe / "recipe.json").read_text())
        definition["build"] = {
            "version_file": "src/fixture/Cargo.toml",
            "version_key": "package.version",
        }
        (recipe / "recipe.json").write_text(json.dumps(definition))
        template = '[component]\nname = "fixture"\nversion = "@VERSION@"\n'
        (recipe / "component.toml.in").write_text(template)
        destination = self.root / "fresh-source-snapshot"
        snapshot(source, destination, "fixture")
        self.assertFalse((recipe / "component.toml").exists())
        self.assertEqual((recipe / "component.toml.in").read_text(), template)
        for prefix in ("", "src/fixture/"):
            contract = destination / prefix / ".anolisa/component.toml"
            self.assertTrue(contract.is_file())
            self.assertFalse(contract.is_symlink())
            self.assertEqual(tomllib.loads(contract.read_text())["component"]["version"], "1.2.3")
            self.assertEqual(contract.stat().st_mode & 0o777, 0o644)
        self.assertFalse((destination / "distribution/components").exists())

    def test_snapshot_flattens_only_selected_component_owned_packaging(self) -> None:
        source, _ = self.snapshot_fixture(
            "src/fixture/fixture.spec.in", "src/fixture/packaging/raw/package.sh"
        )
        component = source / "src/fixture"
        (component / "fixture.spec.in").write_text("fixture spec\n")
        (component / "packaging/raw").mkdir(parents=True)
        (component / "packaging/raw/package.sh").write_text("#!/bin/sh\nexit 0\n")
        (component / "packaging/raw/package.sh").chmod(0o755)
        (component / "private.txt").write_text("unselected source file\n")
        destination = self.root / "component-owned-snapshot"
        snapshot(source, destination, "fixture")
        self.assertTrue((destination / "fixture.spec.in").is_file())
        self.assertEqual((destination / "packaging/raw/package.sh").stat().st_mode & 0o777, 0o755)
        self.assertFalse((destination / "private.txt").exists())

    def test_snapshot_rejects_build_input_outside_component_and_recipe(self) -> None:
        source, _ = self.snapshot_fixture("private.txt")
        (source / "private.txt").write_text("unselected input\n")
        with self.assertRaisesRegex(ValueError, "build input escapes"):
            snapshot(source, self.root / "unsafe-snapshot", "fixture")

    def test_snapshot_rejects_recipe_link_to_unrelated_input(self) -> None:
        source, recipe = self.snapshot_fixture("src/fixture/.anolisa/rpm/fixture.spec.in")
        (source / "private.txt").write_text("unselected input\n")
        (recipe / "rpm").mkdir()
        (recipe / "rpm/fixture.spec.in").symlink_to(source / "private.txt")
        with self.assertRaisesRegex(ValueError, "build input escapes"):
            snapshot(source, self.root / "linked-snapshot", "fixture")

    def test_provider_source_snapshot_can_package_without_the_original_checkout(self) -> None:
        target = self.root / "provider-source"
        target.mkdir()
        snapshot(ROOT, target, "aw-provider-tokenless")
        binary = self.root / "provider-binary"
        binary.write_bytes(b"#!/bin/sh\nexit 0\n")
        binary.chmod(0o755)
        output = self.root / "provider-packages"
        process = subprocess.run(
            [
                sys.executable,
                str(target / "distribution/packaging/component.py"),
                "--component",
                "aw-provider-tokenless",
                "--binary",
                str(binary),
                "--output",
                str(output),
                "--version",
                "0.1.0",
                "--arch",
                "x86_64",
                "--source-commit",
                "a" * 40,
                "--dependency-version",
                "aw-core=0.1.0",
                "--dependency-version",
                "tokenless-core=0.8.8",
            ],
            cwd=self.root,
            capture_output=True,
            text=True,
        )
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertTrue((output / "aw-provider-tokenless/package.json").is_file())

    def test_cli_source_archive_contains_its_shared_installation_library(self) -> None:
        target = self.root / "cli-source"
        target.mkdir()
        (target / "Cargo.toml").write_text((ROOT / "distribution/anolisa/Cargo.toml").read_text())
        snapshot(ROOT, target, "anolisa-cli")
        snapshot(ROOT, target, "anolisa-cli")
        workspace = tomllib.loads((target / "Cargo.toml").read_text())["workspace"]
        self.assertIn("distribution/crates/local-package", workspace["exclude"])
        self.assertIn(
            'path = "distribution/crates/local-package"', (target / "Cargo.toml").read_text()
        )
        engine = target / "distribution/crates/local-package"
        self.assertTrue((engine / "Cargo.toml").is_file())
        self.assertTrue((engine / "src/transaction/engine.rs").is_file())
        self.assertTrue((engine / "runtime/src/lib.rs").is_file())
        self.assertFalse(any(path.name == "target" for path in engine.rglob("*")))

    def test_flat_aw_source_archive_resolves_both_shared_libraries_inside_it(self) -> None:
        original = (ROOT / "src/aw/Cargo.toml").read_text()
        target = self.root / "aw-source"
        target.mkdir()
        (target / "Cargo.toml").write_text(original)
        snapshot(ROOT, target, "aw-core")
        snapshot(ROOT, target, "aw-core")
        dependencies = tomllib.loads((target / "Cargo.toml").read_text())["workspace"][
            "dependencies"
        ]
        for name in ("local-package", "local-package-runtime"):
            with self.subTest(package=name):
                directory = (target / dependencies[name]["path"]).resolve()
                self.assertTrue(directory.is_relative_to(target.resolve()))
                manifest = tomllib.loads((directory / "Cargo.toml").read_text())
                self.assertEqual(manifest["package"]["name"], name)
                self.assertTrue((directory / "src/lib.rs").is_file())
        self.assertEqual((ROOT / "src/aw/Cargo.toml").read_text(), original)

    def test_pip_windows_templates_are_data_and_never_host_executables(self) -> None:
        path = self.root / "site-packages/pip/_vendor/distlib/t32.exe"
        path.parent.mkdir(parents=True)
        path.write_bytes(b"MZ\x90\x00" + b"\0" * 60)
        elf(path, "x86_64", False)
        with self.assertRaises(ValueError):
            elf(path, "x86_64", True)
        renamed = path.with_name("unexpected.exe")
        renamed.write_bytes(path.read_bytes())
        with self.assertRaises(ValueError):
            elf(renamed, "x86_64", False)


if __name__ == "__main__":
    unittest.main()
