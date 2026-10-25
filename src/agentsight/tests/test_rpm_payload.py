"""Validate independent RPM staging without building the Linux executables."""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


COMPONENT = Path(__file__).resolve().parents[1]


class RpmPayloadTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="agentsight-rpm-payload-")
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.source = self.work / "component"
        (self.source / "scripts").mkdir(parents=True)
        for name in ("stage-rpm-payload.sh", "copy-enforcer.py"):
            shutil.copy2(COMPONENT / "scripts" / name, self.source / "scripts" / name)
        for relative in (
            "target/release/agentsight", "scripts/agentsight-start.sh",
            "scripts/agentsight.service", "scripts/agentsight-enforcer.service",
            "agentsight.json", "README.md", "README_zh.md", "LICENSE",
        ):
            path = self.source / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture\n")
        self.enforcer = self.work / "agentsight-enforcer"
        self.enforcer.write_bytes(b"attested prebuilt fixture")
        self.enforcer.chmod(0o755)
        Path(str(self.enforcer) + ".sha256").write_text(
            hashlib.sha256(self.enforcer.read_bytes()).hexdigest()
        )

    def stage(self, contract: Path | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(self.source / "scripts/stage-rpm-payload.sh"), str(self.work / "payload")],
            env={
                **os.environ,
                "ENFORCER_BIN": str(self.enforcer),
                "COMPONENT_CONTRACT": str(contract) if contract is not None else "",
            },
            capture_output=True, text=True,
        )

    def test_detached_component_requires_no_distribution_metadata(self) -> None:
        result = self.stage()
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = self.work / "payload"
        self.assertFalse((payload / "component.toml").exists())
        self.assertEqual((payload / "agentsight-enforcer").read_bytes(), self.enforcer.read_bytes())
        self.assertEqual((payload / "agentsight-enforcer").stat().st_mode & 0o777, 0o755)
        self.assertTrue((payload / "agentsight.service").is_file())

    def test_explicit_integration_contract_is_copied(self) -> None:
        contract = self.work / "integration.toml"
        contract.write_text('[component]\nname = "agentsight"\n')
        result = self.stage(contract)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.work / "payload/component.toml").read_bytes(), contract.read_bytes())

    def test_missing_explicit_contract_fails_before_staging(self) -> None:
        result = self.stage(self.work / "missing.toml")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("optional component contract not found", result.stderr)
        self.assertFalse((self.work / "payload").exists())


if __name__ == "__main__":
    unittest.main()
