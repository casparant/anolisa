#!/usr/bin/env python3
"""Exercise complete fixture assembly, rejection boundaries and launcher contracts.

Synthetic ELF headers are unit-test data, not evidence of building the components.
The Linux integration gate separately installs real AW and ANOLISA binaries.
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))

from assemble import assemble
from build import clean_source, source_versions
from release_common import (
    MAX_DATABASE_BYTES,
    MAX_PREFIX_FILES,
    check_budget,
    composition,
    digest,
    json_bytes,
    read_json,
    relative,
    validate_package,
    version,
)
from stage import (
    adapt_launchers,
    adapt_units,
    mapped_path,
    raw_package,
    unpack_raw,
    write_description,
)

COMMIT = "0123456789abcdef0123456789abcdef01234567"


def synthetic_elf(machine: int = 62, interpreter: str | None = None) -> bytes:
    """Produce parser fixtures without pretending they are runnable binaries."""
    header = bytearray(64)
    header[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<HHI", header, 16, 2, machine, 1)
    struct.pack_into("<H", header, 52, 64)
    if interpreter is None:
        return bytes(header)
    data = interpreter.encode() + b"\0"
    struct.pack_into("<Q", header, 32, 64)
    struct.pack_into("<HH", header, 54, 56, 1)
    program = bytearray(56)
    struct.pack_into("<I", program, 0, 3)
    struct.pack_into("<Q", program, 8, 120)
    struct.pack_into("<Q", program, 32, len(data))
    return bytes(header + program + data)


class CompleteRelease(unittest.TestCase):
    """Use all thirteen package identities, resources and exact dependency edges."""

    def setUp(self) -> None:
        directory = "/private/tmp" if Path("/private/tmp").is_dir() else None
        self.temporary = tempfile.TemporaryDirectory(prefix="anolisa release test-", dir=directory)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.staged = self.root / "packages"
        self.staged.mkdir()
        self.specs = composition(BASE / "linux.json")
        self.versions = {
            spec["package_id"]: f"1.{index}.0" for index, spec in enumerate(self.specs)
        }
        for spec in self.specs:
            payload = self.staged / spec["package_id"] / "payload"
            for name in spec["required"]:
                path = payload / name
                path.parent.mkdir(parents=True, exist_ok=True)
                executable = name.startswith(("bin/", "libexec/"))
                path.write_bytes(synthetic_elf() if executable else b"{}\n")
                path.chmod(0o755 if executable else 0o644)
            self.describe(spec["package_id"])

    def describe(self, package_id: str) -> None:
        spec = next(spec for spec in self.specs if spec["package_id"] == package_id)
        write_description(self.staged / package_id, spec, self.versions, "x86_64", COMMIT)

    def metadata(self, package_id: str) -> dict:
        return json.loads((self.staged / package_id / "package.json").read_bytes())

    def change_metadata(self, package_id: str, document: dict) -> None:
        (self.staged / package_id / "package.json").write_bytes(json_bytes(document))

    def build(self, output: str = "out", **kwargs) -> Path:
        return assemble(
            self.staged,
            self.root / output,
            "4.0.0",
            "4.0.0-fixture",
            COMMIT,
            "x86_64",
            123456789,
            **kwargs,
        )

    def reject(self) -> None:
        with self.assertRaises((ValueError, OSError)):
            self.build()
        self.assertFalse((self.root / "out").exists())

    def test_complete_deterministic_archive_and_checksums(self) -> None:
        first = self.build("first")
        for path in self.staged.rglob("*"):
            os.utime(path, (1, 1), follow_symlinks=False)
        second = self.build("second")
        self.assertEqual(first.read_bytes(), second.read_bytes())
        self.assertFalse((first.parent / "release-index.json").exists())
        self.assertEqual(
            (first.parent / (first.name + ".sha256")).read_text(),
            f"{hashlib.sha256(first.read_bytes()).hexdigest()}  {first.name}\n",
        )
        with tarfile.open(first) as archive:
            members = archive.getmembers()
            self.assertEqual(
                [member.name for member in members], sorted(member.name for member in members)
            )
            self.assertTrue(
                all(
                    member.uid == member.gid == 0 and member.mtime == 123456789
                    for member in members
                )
            )
            top = members[0].name
            release = json.load(archive.extractfile(top + "/release.json"))
            self.assertEqual(len(release["packages"]), 13)
            self.assertEqual(release["entrypoints"]["cosh"], "bin/cosh")
            self.assertEqual(
                set(release),
                {
                    "schema_version",
                    "product_version",
                    "release_id",
                    "source_commit",
                    "os",
                    "arch",
                    "abi",
                    "min_installer_version",
                    "packages",
                    "entrypoints",
                },
            )
            installed = set()
            for item in release["packages"]:
                description = archive.extractfile(top + "/" + item["path"] + "/package.json").read()
                self.assertEqual(
                    hashlib.sha256(description).hexdigest(), item["description_sha256"]
                )
                metadata = json.loads(description)
                self.assertEqual(metadata["version"], self.versions[item["package_id"]])
                self.assertTrue(
                    all(dep["package_id"] in installed for dep in metadata["dependencies"])
                )
                installed.add(item["package_id"])
            checksums = archive.extractfile(top + "/SHA256SUMS").read().decode().splitlines()
            for line in checksums:
                expected, name = line.split("  ", 1)
                self.assertEqual(
                    hashlib.sha256(archive.extractfile(top + "/" + name).read()).hexdigest(),
                    expected,
                )

    def test_explicit_publication_index(self) -> None:
        archive = self.build(artifact_base_url="https://mirror.example/anolisa/releases/4.0.0/")
        index = json.loads((archive.parent / "release-index.json").read_bytes())
        self.assertEqual(index["schema_version"], 1)
        self.assertEqual(index["product_version"], "4.0.0")
        self.assertEqual(
            index["artifacts"],
            [
                {
                    "os": "linux",
                    "arch": "x86_64",
                    "abi": "gnu",
                    "url": "https://mirror.example/anolisa/releases/4.0.0/" + archive.name,
                    "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
                    "size": archive.stat().st_size,
                }
            ],
        )

    def test_invalid_publication_urls_do_not_publish(self) -> None:
        for url in (
            "http://mirror.example",
            "file:///tmp",
            "https://u:p@mirror.example/",
            "https://mirror.example/?secret=1",
            "https://mirror.example/#part",
        ):
            with self.subTest(url=url), self.assertRaises(ValueError):
                self.build(artifact_base_url=url)
        self.assertFalse((self.root / "out").exists())

    def test_existing_output_is_preserved(self) -> None:
        archive = self.build()
        original = archive.read_bytes()
        with self.assertRaises(ValueError):
            self.build()
        self.assertEqual(archive.read_bytes(), original)

    def test_existing_empty_output_is_not_replaced(self) -> None:
        (self.root / "out").mkdir()
        with self.assertRaises(ValueError):
            self.build()
        self.assertEqual(list((self.root / "out").iterdir()), [])

    def test_archive_failure_leaves_no_published_directory(self) -> None:
        with patch("assemble.archive_tree", side_effect=OSError("injected archive failure")):
            self.reject()
        self.assertFalse((self.root / ".out.lock").exists())
        self.build()

    def test_changed_source_during_copy_is_rejected(self) -> None:
        import assemble as module

        original = module.copy_regular
        corrupted = False

        def replace_copy(source: Path, target: Path, mode: int) -> None:
            nonlocal corrupted
            original(source, target, mode)
            if not corrupted and target.name == "aw":
                target.write_bytes(synthetic_elf() + b"changed")
                corrupted = True

        with patch("assemble.copy_regular", side_effect=replace_copy):
            self.reject()

    def test_missing_or_extra_package_is_not_a_complete_release(self) -> None:
        shutil.rmtree(self.staged / "skillfs")
        self.reject()
        (self.staged / "unexpected").mkdir()
        self.reject()

    def test_provider_payload_cannot_duplicate_backend(self) -> None:
        path = self.staged / "aw-provider-tokenless/payload/libexec/aw/providers/tokenless/rtk"
        path.write_bytes(synthetic_elf())
        path.chmod(0o755)
        self.describe("aw-provider-tokenless")
        self.reject()

    def test_missing_required_resource(self) -> None:
        path = self.staged / "cosh-ng/payload/bin/cosh-switch"
        path.unlink()
        self.describe("cosh-ng")
        self.reject()

    def test_payload_tamper_and_unlisted_file(self) -> None:
        path = self.staged / "aw-core/payload/bin/aw"
        path.write_bytes(synthetic_elf() + b"changed")
        self.reject()
        self.describe("aw-core")
        (path.parent / "unlisted").write_text("unlisted")
        self.reject()

    def test_rejects_links_and_special_files(self) -> None:
        path = self.staged / "aw-core/payload/bin/aw"
        path.unlink()
        path.symlink_to("/bin/sh")
        self.reject()
        path.unlink()
        if hasattr(os, "mkfifo"):
            os.mkfifo(path)
            self.reject()

    def test_input_root_symlink_is_not_followed(self) -> None:
        link = self.root / "linked-packages"
        link.symlink_to(self.staged, target_is_directory=True)
        with self.assertRaises(ValueError):
            assemble(link, self.root / "out", "4.0.0", "fixture", COMMIT, "x86_64", 1)

    def test_payload_modes_and_foreign_architectures(self) -> None:
        path = self.staged / "aw-core/payload/bin/aw"
        path.chmod(0o777)
        self.reject()
        path.chmod(0o755)
        path.write_bytes(synthetic_elf(183))
        self.describe("aw-core")
        self.reject()
        path.write_bytes(b"\xcf\xfa\xed\xfe" + b"\0" * 60)
        self.describe("aw-core")
        self.reject()

    def test_musl_interpreter_is_not_gnu(self) -> None:
        path = self.staged / "aw-core/payload/bin/aw"
        path.write_bytes(synthetic_elf(interpreter="/lib/ld-musl-x86_64.so.1"))
        self.describe("aw-core")
        self.reject()
        path.write_bytes(synthetic_elf(interpreter="/lib64/ld-linux-x86-64.so.2"))
        self.describe("aw-core")
        self.build()

    def test_version_source_and_platform_mismatch(self) -> None:
        for key, value in (
            ("source_commit", "f" * 40),
            ("os", "macos"),
            ("arch", "aarch64"),
            ("version", "01.2.3"),
        ):
            with self.subTest(key=key):
                document = self.metadata("aw-core")
                document[key] = value
                self.change_metadata("aw-core", document)
                self.reject()
                self.describe("aw-core")

    def test_exact_dependencies_and_cycles(self) -> None:
        document = self.metadata("anolisa-cli")
        document["dependencies"][0]["version"] = "99.0.0"
        self.change_metadata("anolisa-cli", document)
        self.reject()
        self.describe("anolisa-cli")
        document = self.metadata("aw-core")
        document["dependencies"] = [
            {"package_id": "anolisa-cli", "version": self.versions["anolisa-cli"]}
        ]
        self.change_metadata("aw-core", document)
        self.reject()

    def test_duplicate_ownership_and_file_parent_conflicts(self) -> None:
        for package_id in ("aw-core", "skillfs"):
            path = self.staged / package_id / "payload/share/collision"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"same bytes still need one owner")
            path.chmod(0o644)
            self.describe(package_id)
        self.reject()
        path = self.staged / "skillfs/payload/share/collision"
        path.unlink()
        path.mkdir()
        (path / "child").write_bytes(b"child")
        (path / "child").chmod(0o644)
        self.describe("skillfs")
        self.reject()

    def test_oversized_file_rejected_before_hashing(self) -> None:
        path = self.staged / "aw-core/payload/bin/aw"
        with path.open("wb") as stream:
            stream.truncate(1024**3 + 1)
        self.reject()

    def test_staged_cli_marks_input_as_prebuilt(self) -> None:
        output = self.root / "cli-out"
        result = subprocess.run(
            [
                sys.executable,
                str(BASE / "build.py"),
                "--staged-input",
                str(self.staged),
                "--output-dir",
                str(output),
                "--product-version",
                "4.0.0",
                "--release-id",
                "fixture",
                "--source-commit",
                COMMIT,
                "--arch",
                "x86_64",
                "--epoch",
                "1",
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        with tarfile.open(next(output.glob("*.tar.gz"))) as archive:
            top = archive.getmembers()[0].name
            evidence = json.load(archive.extractfile(top + "/provenance/build-input.json"))
            self.assertEqual(evidence["input_kind"], "staged_packages")


class ValidationAndLaunchers(unittest.TestCase):
    """Check attack surfaces and the handful of intentional layout translations."""

    def setUp(self) -> None:
        directory = "/private/tmp" if Path("/private/tmp").is_dir() else None
        self.temporary = tempfile.TemporaryDirectory(prefix="release launcher $-", dir=directory)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def test_budgets_include_prefix_and_journal_limits(self) -> None:
        check_budget([], {f"bin/f{index}": 0 for index in range(MAX_PREFIX_FILES)})
        for sizes in (
            {f"bin/f{index}": 0 for index in range(MAX_PREFIX_FILES + 1)},
            {"bin/big": 4 * 1024**3 + 1},
        ):
            with self.assertRaises(ValueError):
                check_budget([], sizes)
        with self.assertRaises(ValueError):
            check_budget([{"large": "x" * MAX_DATABASE_BYTES}], {})
        with self.assertRaises(ValueError):
            check_budget(
                [], {f"share/{'x' * 200}/{index}/" + "y" * 1000: 0 for index in range(16000)}
            )

    def test_description_json_duplicates_and_size(self) -> None:
        path = self.root / "package.json"
        path.write_text('{"schema_version":1,"schema_version":2}')
        with self.assertRaises(ValueError):
            read_json(path)
        path.write_bytes(b" " * (1024**2 + 1))
        with self.assertRaises(ValueError):
            read_json(path)

    def test_relative_path_and_semver_boundaries(self) -> None:
        for path in (
            "/bin/evil",
            "bin/../evil",
            "bin//evil",
            "bin/./evil",
            "bin/evil\\path",
            "bin/line\nbreak",
            "lib/unsupported",
        ):
            with self.subTest(path=path), self.assertRaises(ValueError):
                relative(path, payload=True)
        for value in ("01.2.3", "1.2", "1.2.3-01", "v1.2.3", "1.2.3-"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                version(value)
        self.assertEqual(version("1.2.3-preview.1+build.01"), "1.2.3-preview.1+build.01")

    def test_raw_archive_cannot_escape_or_follow_links(self) -> None:
        for index, name in enumerate(("../outside", "/absolute", "bin/link")):
            archive = self.root / f"unsafe{index}.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                info = tarfile.TarInfo(name)
                if name == "bin/link":
                    info.type = tarfile.SYMTYPE
                    info.linkname = "/bin/sh"
                    output.addfile(info)
                else:
                    info.size = 1
                    output.addfile(info, io.BytesIO(b"x"))
            with self.assertRaises(ValueError):
                unpack_raw(archive, self.root / f"raw{index}")
        self.assertFalse((self.root / "outside").exists())

    def test_only_known_component_layouts_are_mapped(self) -> None:
        self.assertEqual(
            mapped_path("sec-core", "lib/anolisa/sec-core/python/a.py"),
            "libexec/anolisa/sec-core/python/a.py",
        )
        self.assertEqual(
            mapped_path("tokenless", "adapters/openclaw/package.json"),
            "share/anolisa/adapters/tokenless/openclaw/package.json",
        )
        self.assertEqual(
            mapped_path("agent-memory", "adapters/agent-memory/manifest.json"),
            "share/anolisa/adapters/agent-memory/manifest.json",
        )
        with self.assertRaises(ValueError):
            mapped_path("aw-core", "etc/systemd/unapproved.service")

    def test_rtk_launcher_binds_the_private_binary(self) -> None:
        (self.root / "bin").mkdir()
        private = self.root / "libexec/anolisa/tokenless/rtk"
        private.parent.mkdir(parents=True)
        private.write_text('#!/bin/sh\nprintf "private:%s\\n" "$1"\n')
        private.chmod(0o755)
        (self.root / "bin/tokenless").write_text("#!/bin/sh\nexit 0\n")
        (self.root / "bin/tokenless").chmod(0o755)
        adapt_launchers(self.root, "tokenless")
        result = subprocess.run(
            [str(self.root / "bin/rtk"), "argument with spaces"],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(result.stdout, "private:argument with spaces\n")

    def test_tokenless_launcher_pins_rules_and_helper_search(self) -> None:
        (self.root / "bin").mkdir()
        binary = self.root / "bin/tokenless"
        binary.write_text(
            '#!/bin/sh\nprintf "%s\n%s\n%s\n" "$TOKENLESS_TOOL_READY_SPEC" '
            '"$TOKENLESS_ENV_FIX_SCRIPT" "$PATH"\n'
        )
        binary.chmod(0o755)
        adapt_launchers(self.root, "tokenless")
        result = subprocess.run([str(binary)], capture_output=True, text=True, check=True)
        lines = result.stdout.splitlines()
        self.assertEqual(
            lines[0],
            str(self.root / "share/anolisa/adapters/tokenless/common/tool-ready-spec.json"),
        )
        self.assertEqual(
            lines[1],
            str(self.root / "share/anolisa/adapters/tokenless/common/tokenless-env-fix.sh"),
        )
        self.assertTrue(lines[2].startswith(str(self.root / "bin") + ":"))
        self.assertTrue((self.root / "libexec/anolisa/tokenless/tokenless").is_file())

    def ws_ckpt_resources(self, payload: Path) -> None:
        source = BASE.parents[3] / "src/ws-ckpt"
        plugin_files = {
            "openclaw": ("package.json", "openclaw.plugin.json", "dist/src/index.js"),
            "hermes": ("plugin.yaml", "__init__.py"),
        }
        for framework, names in plugin_files.items():
            helper = payload / "share/anolisa/adapters/ws-ckpt" / framework / "lib-discover.sh"
            helper.parent.mkdir(parents=True)
            shutil.copyfile(source / "scripts" / framework / "lib-discover.sh", helper)
            plugin = payload / "share/anolisa/runtime/ws-ckpt/plugins" / framework
            for name in names:
                path = plugin / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
        skill = payload / "share/anolisa/runtime/skills/ws-ckpt"
        skill.mkdir(parents=True)
        (skill / "SKILL.md").write_text("fixture")
        adapt_launchers(payload, "ws-ckpt")

    def check_ws_ckpt_discovery(self, resource_path: Path, actual_resources: Path) -> None:
        for framework in ("openclaw", "hermes"):
            with self.subTest(framework=framework, resource_path=resource_path):
                helper = resource_path / "anolisa/adapters/ws-ckpt" / framework / "lib-discover.sh"
                environment = os.environ.copy()
                environment["ANOLISA_RELEASE_PREFIX"] = "/bound program/releases/selected"
                result = subprocess.run(
                    [
                        "bash",
                        "-c",
                        'set -eu; source "$1"; find_plugin_src "$2"; find_skill_src; '
                        'printf "%s\\n" "$ANOLISA_RELEASE_PREFIX"',
                        "fixture",
                        str(helper),
                        framework,
                    ],
                    env=environment,
                    capture_output=True,
                    text=True,
                    check=True,
                )
                self.assertEqual(
                    result.stdout.splitlines(),
                    [
                        str(actual_resources / "anolisa/runtime/ws-ckpt/plugins" / framework),
                        str(actual_resources / "anolisa/runtime/skills/ws-ckpt"),
                        environment["ANOLISA_RELEASE_PREFIX"],
                    ],
                )

    def test_ws_ckpt_real_discovery_binds_release_resources(self) -> None:
        self.ws_ckpt_resources(self.root)
        self.check_ws_ckpt_discovery(self.root / "share", self.root / "share")

    def test_ws_ckpt_discovery_uses_separate_resources_through_alias_and_direct_paths(self) -> None:
        payload = self.root / "program/releases/selected"
        self.ws_ckpt_resources(payload)
        resources = self.root / "other volume/nested/portable/data/releases/selected"
        resources.parent.mkdir(parents=True)
        shutil.move(payload / "share", resources)
        (payload / "share").symlink_to(resources, target_is_directory=True)
        for path in (payload / "share", resources):
            self.check_ws_ckpt_discovery(path, resources)

    def test_service_assets_become_uninstalled_bindir_templates(self) -> None:
        sources = {
            "agentsight": [
                (
                    "src/agentsight/scripts/agentsight.service",
                    "share/anolisa/agentsight/agentsight.service",
                ),
                (
                    "src/agentsight/scripts/agentsight-enforcer.service",
                    "share/anolisa/agentsight/agentsight-enforcer.service",
                ),
            ],
            "agent-memory": [
                (
                    "src/agent-memory/config/systemd/anolisa-memory@.service",
                    "share/anolisa/agent-memory/anolisa-memory@.service",
                )
            ],
            "ws-ckpt": [
                ("src/ws-ckpt/src/systemd/ws-ckpt.service", "share/anolisa/ws-ckpt/ws-ckpt.service")
            ],
        }
        for package_id, paths in sources.items():
            for origin, relative in paths:
                target = self.root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(BASE.parents[3] / origin, target)
            adapt_units(self.root, package_id)
            for _, relative in paths:
                content = (self.root / relative).read_text()
                self.assertIn("{bindir}/", content)
                self.assertNotIn("ExecStart=/usr/", content)

    def test_real_cosh_stage_carries_switch_and_private_runtime_assets(self) -> None:
        source = BASE.parents[3]
        spec = next(
            item for item in composition(BASE / "linux.json") if item["package_id"] == "cosh-ng"
        )
        component_version = source_versions(source, [spec])["cosh-ng"]
        binaries = self.root / "binaries"
        binaries.mkdir()
        names = ("cosh-cli", "cosh-core", "cosh-gateway", "cosh-shell")
        for name in names:
            (binaries / name).write_bytes(synthetic_elf())
            (binaries / name).chmod(0o755)
        metadata = binaries / "cosh-ng-build.toml"
        metadata.write_text(
            f'version = "{component_version}"\ntarget_os = "linux"\ntarget_arch = "x86_64"\n\n[binaries]\n'
            + "".join(f'{name} = "{digest(binaries / name)}"\n' for name in names)
        )
        raw = self.root / "cosh-raw"
        env = os.environ.copy()
        env.update(
            {
                "PATH": str(Path(sys.executable).parent) + ":" + env.get("PATH", ""),
                "BIN_DIR": str(binaries),
                "COSH_NG_BUILD_METADATA": str(metadata),
                "DESTDIR": str(raw),
                "TARGET_OS": "linux",
                "TARGET_ARCH": "x86_64",
            }
        )
        result = subprocess.run(
            ["bash", str(source / "src/cosh-ng/packaging/raw/package.sh"), "stage"],
            env=env,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        target = self.root / "cosh-package"
        raw_package(raw, target, spec, source)
        write_description(target, spec, {"cosh-ng": component_version}, "x86_64", COMMIT)
        validate_package(target, spec, "x86_64", COMMIT)
        self.assertTrue((target / "payload/bin/cosh-switch").is_file())

    def test_real_static_skills_package_preserves_tracked_resources(self) -> None:
        source = BASE.parents[3]
        spec = next(
            item for item in composition(BASE / "linux.json") if item["package_id"] == "os-skills"
        )
        component_version = source_versions(source, [spec])["os-skills"]
        output = self.root / "skills-artifacts"
        result = subprocess.run(
            [
                sys.executable,
                str(source / "src/os-skills/packaging/raw/package.py"),
                "--source-dir",
                str(source / "src/os-skills"),
                "--output-dir",
                str(output),
                "--version",
                component_version,
                "--target-arch",
                "x86_64",
                "--epoch",
                "1",
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        raw = self.root / "skills-raw"
        unpack_raw(next(output.glob("*.tar.gz")), raw)
        target = self.root / "skills-package"
        raw_package(raw, target, spec, source)
        write_description(target, spec, {"os-skills": component_version}, "x86_64", COMMIT)
        document = validate_package(target, spec, "x86_64", COMMIT)
        self.assertGreater(len(document["files"]), 100)

    def test_cosh_switch_does_not_swap_host_packages(self) -> None:
        (self.root / "bin").mkdir()
        adapt_launchers(self.root, "cosh-ng")
        result = subprocess.run(
            [str(self.root / "bin/cosh-switch")], capture_output=True, text=True
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("does not modify host RPM packages", result.stderr)

    def test_sec_core_python_launcher_uses_relocated_runtime(self) -> None:
        (self.root / "bin").mkdir()
        source = BASE.parents[3] / "src/agent-sec-core/packaging/raw/assets/bin/agent-sec-python"
        shutil.copyfile(source, self.root / "bin/agent-sec-python")
        (self.root / "bin/agent-sec-python").chmod(0o755)
        python = self.root / "libexec/anolisa/sec-core/python3.11/runtime/bin/python3.11"
        python.parent.mkdir(parents=True)
        python.write_text(
            '#!/bin/sh\nif [ "$1" = -c ]; then echo 3.11.6; '
            'else printf "%s:%s\\n" "$PYTHONHOME" "$1"; fi\n'
        )
        python.chmod(0o755)
        adapt_launchers(self.root, "sec-core")
        result = subprocess.run(
            [str(self.root / "bin/agent-sec-python"), "hello"],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(result.stdout, f"{python.parent.parent}:hello\n")

    def test_install_launcher_executes_bundled_cli_with_exact_arguments(self) -> None:
        cli = self.root / "packages/anolisa-cli/payload/bin/anolisa"
        cli.parent.mkdir(parents=True)
        cli.write_text('#!/bin/sh\nprintf "%s\\0" "$@"\nexit 23\n')
        cli.chmod(0o755)
        shutil.copyfile(BASE / "install.sh", self.root / "install.sh")
        (self.root / "install.sh").chmod(0o755)
        result = subprocess.run(
            [str(self.root / "install.sh"), "--scope", "user", "two words"], capture_output=True
        )
        self.assertEqual(result.returncode, 23)
        self.assertEqual(
            result.stdout.split(b"\0")[:-1],
            [
                b"release",
                b"install",
                b"--bundle",
                str(self.root).encode(),
                b"--scope",
                b"user",
                b"two words",
            ],
        )

    def test_source_build_versions_are_component_owned(self) -> None:
        versions = source_versions(BASE.parents[3], composition(BASE / "linux.json"))
        self.assertEqual(len(versions), 13)
        self.assertNotEqual(versions["aw-core"], versions["anolisa-cli"])
        self.assertEqual(versions["aw-provider-tokenless"], "0.1.0")

    def test_source_clean_gate_rejects_unpublished_changes(self) -> None:
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "-C", str(self.root), "config", "user.name", "Fixture"], check=True)
        subprocess.run(
            ["git", "-C", str(self.root), "config", "user.email", "fixture@example.invalid"],
            check=True,
        )
        path = self.root / "source"
        path.write_text("source\n")
        subprocess.run(["git", "-C", str(self.root), "add", "source"], check=True)
        subprocess.run(
            [
                "git",
                "-C",
                str(self.root),
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-qm",
                "fixture",
            ],
            check=True,
        )
        original = clean_source(self.root, [])
        output = self.root / "output"
        output.mkdir()
        (output / "artifact").write_bytes(b"output")
        self.assertEqual(clean_source(self.root, [output]), original)
        with self.assertRaises(ValueError):
            clean_source(self.root, [])
        path.write_text("modified\n")
        with self.assertRaises(ValueError):
            clean_source(self.root, [output])


if __name__ == "__main__":
    unittest.main()
