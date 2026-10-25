#!/usr/bin/env python3
"""Keep ANOLISA Source0, Source1 and component-local metadata at one committed revision."""

import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class AnolisaRpmSnapshot(unittest.TestCase):
    def test_dirty_worktree_cannot_change_the_committed_source_or_recipe(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "repository"
            root.mkdir()

            def write(name: str, text: str, executable: bool = False) -> Path:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
                if executable:
                    path.chmod(0o755)
                return path

            source = write(
                "distribution/anolisa/Cargo.toml", '[workspace.package]\nversion = "1.2.3"\n'
            )
            write("LICENSE", "fixture license\n")
            spec = write(
                "distribution/anolisa/anolisa.spec.in",
                "Name: anolisa\nVersion: @VERSION@\n# committed recipe\n",
            )
            write(
                "distribution/anolisa/.anolisa/recipe.json",
                json.dumps(
                    {
                        "id": "anolisa-cli",
                        "source_dir": "distribution/anolisa",
                        "rpm_spec": "distribution/anolisa/anolisa.spec.in",
                    }
                ),
            )
            write("distribution/release/linux.json", "{}\n")
            write(
                "providers/fixture/.anolisa/recipe.json",
                '{"id":"fixture-provider","source_dir":"providers/fixture"}\n',
            )
            tool = write(
                "distribution/packaging/snapshot.py",
                (ROOT / "distribution/packaging/snapshot.py").read_text(),
            )
            write(
                "distribution/packaging/recipes.py",
                (ROOT / "distribution/packaging/recipes.py").read_text(),
            )
            write("scripts/rpm-build.sh", (ROOT / "scripts/rpm-build.sh").read_text(), True)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(root),
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-qm",
                    "fixture",
                ],
                check=True,
            )
            source.write_text('[workspace.package]\nversion = "9.9.9"\n')
            spec.write_text("Name: anolisa\nVersion: 9.9.9\n# dirty recipe\n")
            tool.write_text("raise SystemExit('dirty snapshot tool must never execute')\n")
            tools = Path(temporary) / "tools"
            tools.mkdir()
            cargo = tools / "cargo"
            cargo.write_text(
                '#!/bin/sh\nset -eu\nfor argument in "$@"; do destination="$argument"; done\nmkdir -p "$destination"\ncp Cargo.toml "$destination/committed-cargo.toml"\n'
            )
            cargo.chmod(0o755)
            rpm = tools / "rpmbuild"
            rpm.write_text("#!/bin/sh\nexit 0\n")
            rpm.chmod(0o755)
            result = subprocess.run(
                ["bash", str(root / "scripts/rpm-build.sh"), "anolisa"],
                cwd=root,
                env={
                    **os.environ,
                    "PATH": str(tools) + os.pathsep + os.environ["PATH"],
                    "RPMBUILD": str(rpm),
                    "COPYFILE_DISABLE": "1",
                },
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            sources = root / "scripts/rpmbuild/SOURCES"
            with tarfile.open(sources / "anolisa-1.2.3.tar.gz") as archive:
                self.assertEqual(
                    archive.extractfile("anolisa/Cargo.toml").read(),
                    b'[workspace.package]\nversion = "1.2.3"\n',
                )
                member = archive.getmember("anolisa/anolisa.spec.in")
                self.assertTrue(member.isfile())
                self.assertIn(b"committed recipe", archive.extractfile(member).read())
                metadata = json.loads(archive.extractfile("anolisa/.anolisa/recipe.json").read())
                self.assertEqual(metadata["id"], "anolisa-cli")
                self.assertEqual(metadata["source_dir"], "distribution/anolisa")
            with tarfile.open(sources / "anolisa-1.2.3-vendor.tar.gz") as archive:
                self.assertEqual(
                    archive.extractfile("vendor/committed-cargo.toml").read(),
                    b'[workspace.package]\nversion = "1.2.3"\n',
                )
            self.assertIn("9.9.9", source.read_text())
            self.assertIn("dirty recipe", spec.read_text())


if __name__ == "__main__":
    unittest.main()
