"""Exercise the raw package boundary without executing fixture ELF files."""

import hashlib
import json
import os
import shutil
import struct
import subprocess
import tarfile
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGER = ROOT / "packaging/raw/package.py"


class RawPackaging(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.work = Path(self.tmp.name)
        self.source = self.work / "source"
        self.source.mkdir()
        for name in ("config", "adapters", "packaging"):
            shutil.copytree(
                ROOT / name,
                self.source / name,
                ignore=shutil.ignore_patterns("node_modules", "dist", "coverage"),
            )
        shutil.copy2(ROOT / "Cargo.toml", self.source / "Cargo.toml")
        self.packager = self.source / "packaging/raw/package.py"
        self.version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"][
            "version"
        ]
        self.bin = self.work / "private-bin"
        self.bin.mkdir()
        self.plugin = self.work / "private-plugin"
        self.plugin.mkdir()
        (self.plugin / "index.js").write_text("export default {};\n")
        (self.plugin / "build-meta.json").write_text("{}")
        self.metadata = self.work / "build.toml"
        self.prepare("x86_64")

    def prepare(self, arch: str) -> None:
        blob = bytearray(64)
        blob[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", blob, 18, {"x86_64": 62, "aarch64": 183}[arch])
        (self.bin / "agent-memory").write_bytes(blob)
        (self.bin / "agent-memory").chmod(0o755)
        plugin_hash = hashlib.sha256(
            (self.plugin / "index.js").read_bytes()
        ).hexdigest()
        self.metadata.write_text(
            f'version = "{self.version}"\ntarget_os = "linux"\ntarget_arch = "{arch}"\n'
            f'[binaries]\nagent-memory = "{hashlib.sha256(blob).hexdigest()}"\n'
            f'[plugin]\nsha256 = "{plugin_hash}"\n'
        )

    def run_package(
        self, arch: str = "x86_64", success: bool = True, contract: Path | None = None
    ) -> subprocess.CompletedProcess:
        result = subprocess.run(
            [
                "python3",
                str(self.packager),
                "--source-dir",
                str(self.source),
                "--bin-dir",
                str(self.bin),
                "--plugin-dir",
                str(self.plugin),
                "--metadata",
                str(self.metadata),
                "--target-arch",
                arch,
                "--output-dir",
                str(self.work / "out"),
                "--epoch",
                "1700000000",
                *(["--contract", str(contract)] if contract is not None else []),
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode == 0, success, result.stderr)
        return result

    def test_architectures_layout_and_reproducibility(self) -> None:
        for arch in ("x86_64", "aarch64"):
            self.prepare(arch)
            archive = Path(self.run_package(arch).stdout.strip())
            first = archive.read_bytes()
            self.run_package(arch)
            self.assertEqual(first, archive.read_bytes())
            with tarfile.open(archive) as tar:
                files = {
                    m.name.removeprefix("./"): tar.extractfile(m).read()
                    for m in tar
                    if m.isfile()
                }
                self.assertEqual(len(files), 11)
                self.assertEqual(
                    files["bin/agent-memory"], (self.bin / "agent-memory").read_bytes()
                )
                self.assertEqual(
                    files["share/agent-memory/adapters/openclaw/dist/index.js"],
                    (self.plugin / "index.js").read_bytes(),
                )
                self.assertNotIn(".anolisa/component.toml", files)
                self.assertIn("share/agent-memory/mcp-servers/agent-memory.json", files)
                self.assertFalse(any(name.startswith("share/anolisa/") for name in files))
                for item in tar.getmembers():
                    self.assertEqual(
                        (item.uid, item.gid, item.mtime), (0, 0, 1700000000)
                    )
                    executable = (
                        item.name.endswith(".sh") or item.name == "./bin/agent-memory"
                    )
                    self.assertEqual(
                        item.mode, 0o755 if item.isdir() or executable else 0o644
                    )

    def test_optional_contract_is_validated_and_embedded(self) -> None:
        contract = self.work / "integration.toml"
        contract.write_text(
            f'[component]\nname = "agent-memory"\nversion = "{self.version}"\n'
            '[component.platform]\nos = ["linux"]\narch = ["x86_64"]\n'
        )
        archive = Path(self.run_package(contract=contract).stdout.strip())
        with tarfile.open(archive) as tar:
            self.assertEqual(tar.extractfile("./.anolisa/component.toml").read(), contract.read_bytes())
        contract.write_text(contract.read_text().replace(self.version, "99.0.0"))
        self.assertIn("contract identity", self.run_package(success=False, contract=contract).stderr)

    def test_source_archive_is_self_contained_without_distribution_checkout(self) -> None:
        for name in ("src", "examples", "tests"):
            shutil.copytree(ROOT / name, self.source / name)
        for name in ("Cargo.lock", "Makefile", "agent-memory.spec.in", "CHANGELOG.md", "LICENSE", ".gitignore"):
            shutil.copyfile(ROOT / name, self.source / name)
        plugin = self.source / "adapters/agent-memory/openclaw/dist"
        plugin.mkdir()
        shutil.copyfile(self.plugin / "index.js", plugin / "index.js")
        tools = self.work / "tools"
        tools.mkdir()
        cargo = tools / "cargo"
        cargo.write_text('#!/bin/sh\n[ "$1" = vendor ] || exit 42\nmkdir -p "$2"\n')
        cargo.chmod(0o755)
        subprocess.run(
            ["make", "-C", str(self.source), "-o", "clean", "-o", "build-openclaw-plugin", "dist"],
            env={**os.environ, "PATH": f"{tools}{os.pathsep}{os.environ['PATH']}"},
            check=True, capture_output=True, text=True,
        )
        with tarfile.open(self.source / "dist" / f"agent-memory-{self.version}.tar.gz") as tar:
            members = {member.name: member for member in tar}
            prefix = f"agent-memory-{self.version}/"
            for name in ("agent-memory.spec.in", "packaging/raw/package.py", "packaging/raw/verify-release.py"):
                self.assertTrue(members[prefix + name].isfile(), name)
            self.assertFalse(any(".anolisa" in Path(name).parts for name in members))
            self.assertFalse(any(member.issym() for member in members.values()))

    def test_missing_and_stale_metadata(self) -> None:
        self.metadata.unlink()
        self.run_package(success=False)
        self.prepare("x86_64")
        self.metadata.write_text(
            self.metadata.read_text().replace(self.version, "0.0.0")
        )
        self.assertIn("version", self.run_package(success=False).stderr)

    def test_tampered_binary_and_plugin(self) -> None:
        with (self.bin / "agent-memory").open("ab") as stream:
            stream.write(b"replacement")
        self.assertIn("hash", self.run_package(success=False).stderr)
        self.prepare("x86_64")
        (self.plugin / "index.js").write_text("replacement")
        self.assertIn("plugin hash", self.run_package(success=False).stderr)

    def test_missing_outputs(self) -> None:
        (self.bin / "agent-memory").unlink()
        self.run_package(success=False)
        self.prepare("x86_64")
        (self.plugin / "index.js").unlink()
        self.run_package(success=False)

    def test_wrong_architecture(self) -> None:
        self.assertIn("architecture", self.run_package("aarch64", success=False).stderr)
        self.run_package("riscv64", success=False)

    def test_version_drift_fails_without_repair(self) -> None:
        path = self.source / "config/mcp-server.json"
        data = json.loads(path.read_text())
        data["version"] = "0.0.0"
        path.write_text(json.dumps(data))
        self.assertIn("version", self.run_package(success=False).stderr)
        self.assertEqual(json.loads(path.read_text())["version"], "0.0.0")

    def test_shared_output_is_not_used(self) -> None:
        shared = self.source / "target/release"
        shared.mkdir(parents=True)
        (shared / "agent-memory").write_text("stale host binary")
        self.run_package()
        (self.bin / "agent-memory").unlink()
        self.run_package(success=False)


if __name__ == "__main__":
    unittest.main()
