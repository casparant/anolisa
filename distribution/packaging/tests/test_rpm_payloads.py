"""Exercise RPM install sections and prove staged payload ownership without compiling."""

from __future__ import annotations

import glob
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


class RpmPayloadOwnership(unittest.TestCase):
    @unittest.skipUnless(
        shutil.which("flock"), "AgentSight attestation integration requires Linux flock"
    )
    def test_agentsight_attested_rpm_integration(self) -> None:
        result = subprocess.run(
            ["bash", str(ROOT / "distribution/packaging/tests/test-agentsight-rpm-integration.sh")],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="anolisa-rpm-payload-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.buildroot = self.root / "buildroot"

    def copy_source(self, component: str, directories: tuple[str, ...]) -> None:
        origin = ROOT / "src" / component
        for name in directories:
            path = origin / name
            if path.is_dir():
                shutil.copytree(
                    path,
                    self.source / name,
                    ignore=shutil.ignore_patterns(
                        "node_modules", "target", "__pycache__", "tests", ".git"
                    ),
                )
        for name in ("LICENSE", "README.md", "CHANGELOG.md", "component.toml"):
            if (origin / name).is_file():
                shutil.copyfile(origin / name, self.source / name)
        for template in list(self.source.rglob("*.in")):
            output = template.with_suffix("")
            if not output.exists():
                output.write_text(template.read_text().replace("@VERSION@", "0.1.0"))
        contract = origin / ".anolisa/component.toml.in"
        (self.source / ".anolisa").mkdir(exist_ok=True)
        shutil.copyfile(contract, self.source / ".anolisa/component.toml")

    def fixture(self, name: str, data: bytes = b"fixture\n") -> None:
        path = self.source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755)

    def macros(self, name: str) -> dict[str, str]:
        return {
            "buildroot": str(self.buildroot),
            "name": name,
            "version": "0.1.0",
            "_bindir": "/usr/bin",
            "_localbindir": "/usr/local/bin",
            "_libexecdir": "/usr/libexec",
            "_libexecdir_cosh": "/usr/libexec/anolisa/cosh-ng",
            "_datadir": "/usr/share",
            "_sysconfdir": "/etc",
            "_unitdir": "/usr/lib/systemd/system",
            "_userunitdir": "/usr/lib/systemd/user",
            "_tmpfilesdir": "/usr/lib/tmpfiles.d",
            "_docdir": "/usr/share/doc",
        }

    def expanded(self, text: str, values: dict[str, str]) -> str:
        for key, value in values.items():
            text = text.replace("%{" + key + "}", value)
        return text

    def conditionals(self, text: str, integration: bool) -> str:
        """Select the same optional metadata branch as rpmbuild's --with option."""
        active = [True]
        output: list[str] = []
        for line in text.splitlines():
            if line.startswith("%bcond_with "):
                self.assertEqual(line, "%bcond_with anolisa_integration")
            elif line.startswith("%if "):
                self.assertEqual(line, "%if %{with anolisa_integration}")
                active.append(active[-1] and integration)
            elif line == "%else":
                self.assertGreater(len(active), 1)
                active[-1] = active[-2] and not active[-1]
            elif line == "%endif":
                self.assertGreater(len(active), 1)
                active.pop()
            elif active[-1]:
                output.append(
                    line.replace(
                        "%{?with_anolisa_integration:.anolisa/component.toml}",
                        ".anolisa/component.toml" if integration else "",
                    )
                )
        self.assertEqual(len(active), 1, "unclosed RPM conditional")
        return "\n".join(output) + "\n"

    def check_payload(
        self,
        component: str,
        filename: str | None = None,
        prelude: str = "",
        integration: bool = False,
    ) -> dict[str, set[str]]:
        source_name = "agent-sec-core" if component == "sec-core" else component
        path = ROOT / "src" / source_name / (filename or component + ".spec.in")
        spec = self.conditionals(path.read_text(), integration)
        name = re.search(r"(?m)^Name:\s+(\S+)", spec).group(1)
        values = self.macros(name)
        install = re.split(r"(?m)^%install\n", spec, maxsplit=1)[1]
        install = re.split(r"(?m)^%(?:files|changelog|post|preun|postun)\b", install, maxsplit=1)[0]
        self.buildroot.mkdir(exist_ok=True)
        result = subprocess.run(
            ["bash", "-e", "-u", "-c", prelude + self.expanded(install, values)],
            cwd=self.source,
            env={**os.environ, "RPM_BUILD_ROOT": str(self.buildroot)},
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        sections: dict[str, set[str]] = {}
        current = None
        for line in spec.splitlines():
            if line.startswith("%files"):
                fields = line.split()
                current = fields[2] if len(fields) > 2 and fields[1] == "-n" else name
                sections[current] = set()
                continue
            if current is None:
                continue
            if re.match(
                r"^%(?:package|description|prep|build|install|post|preun|postun|posttrans|changelog)\b",
                line,
            ):
                current = None
                continue
            if not line or line.startswith(("#", "%defattr", "%dir")):
                continue
            if line.startswith("%exclude "):
                exclusion = self.expanded(line.removeprefix("%exclude "), values).lstrip("/")
                sections[current] = {path for path in sections[current] if path != exclusion}
                continue
            # RPM installs relative license/doc arguments itself; absolute payload paths must already exist.
            pattern = re.sub(r"^%(?:license|doc)\s+", "", line)
            while re.match(r"^%(?:attr|config)\(", pattern):
                pattern = re.sub(r"^%(?:attr|config)\([^)]*\)\s*", "", pattern)
            pattern = self.expanded(pattern, values)
            if not pattern.startswith("/"):
                continue
            matches = glob.glob(str(self.buildroot) + pattern.rstrip("/"))
            self.assertTrue(matches, f"{current}: declared RPM path is missing: {pattern}")
            for match in matches:
                candidate = Path(match)
                selected = candidate.rglob("*") if candidate.is_dir() else [candidate]
                sections[current].update(
                    str(item.relative_to(self.buildroot))
                    for item in selected
                    if item.is_file() or item.is_symlink()
                )
        staged = {
            str(path.relative_to(self.buildroot))
            for path in self.buildroot.rglob("*")
            if path.is_file() or path.is_symlink()
        }
        for path in staged:
            owners = [package for package, files in sections.items() if path in files]
            self.assertEqual(len(owners), 1, f"{component}/{path}: owners={owners}")
        return sections

    def test_tokenless_subpackages_own_canonical_and_compatibility_payloads(
        self,
    ) -> None:
        self.copy_source("tokenless", ("adapters", "docs"))
        self.fixture("target/release/tokenless")
        self.fixture("third_party/rtk/target/release/rtk")
        self.fixture("adapters/tokenless/openclaw/dist/index.js")
        self.fixture("adapters/tokenless/dsh/dist/index.js")
        for integration in (True, False):
            if not integration:
                (self.source / ".anolisa/component.toml").unlink()
            with self.subTest(integration=integration):
                owners = self.check_payload("tokenless", integration=integration)
                metadata = "usr/share/anolisa/components/tokenless/component.toml"
                self.assertEqual(metadata in owners["tokenless-core"], integration)
        self.assertIn("usr/bin/tokenless", owners["tokenless-core"])
        self.assertFalse(
            any("/hermes/" in path or "/openclaw/" in path for path in owners["tokenless-core"])
        )
        self.assertIn(
            "usr/share/tokenless/adapters/hermes/plugin.yaml",
            owners["tokenless-plugins"],
        )
        self.assertIn(
            "usr/share/anolisa/adapters/tokenless/hermes/plugin.yaml",
            owners["tokenless-plugins"],
        )

    def test_ws_ckpt_keeps_backend_plugins_and_skills_separate(self) -> None:
        self.copy_source("ws-ckpt", ("src/skills", "src/plugins", "scripts", "src/systemd"))
        shutil.copyfile(
            ROOT / "src/ws-ckpt/src/config.toml.sample",
            self.source / "src/config.toml.sample",
        )
        self.fixture("src/target/release/ws-ckpt")
        self.fixture("src/plugins/openclaw/dist/src/index.js")
        owners = self.check_payload("ws-ckpt")
        self.assertIn("etc/ws-ckpt/config.toml", owners["ws-ckpt-core"])
        self.assertIn("usr/share/ws-ckpt/skills/ws-ckpt/SKILL.md", owners["ws-ckpt-skills"])

    def test_skillfs_separates_the_mount_workflow(self) -> None:
        self.copy_source("skillfs", ("docs/skills",))
        self.fixture("target/release/skillfs")
        owners = self.check_payload("skillfs")
        self.assertIn("usr/local/bin/skillfs", owners["skillfs-core"])
        self.assertTrue(owners["skillfs-skills"])

    def test_memory_plugin_is_optional_to_mcp_backend(self) -> None:
        self.copy_source("agent-memory", ("adapters", "config"))
        self.fixture("target/release/agent-memory")
        self.fixture("adapters/agent-memory/openclaw/dist/index.js")
        owners = self.check_payload("agent-memory")
        self.assertIn("usr/bin/agent-memory", owners["agent-memory-core"])
        self.assertFalse(any("/adapters/" in path for path in owners["agent-memory-core"]))

    def test_ktuner_native_rpm_needs_no_distribution_metadata(self) -> None:
        self.copy_source("ktuner", ())
        self.fixture("target/release/ktuner")
        (self.source / "component.toml").unlink()
        owners = self.check_payload("ktuner")
        self.assertIn("usr/local/bin/ktuner", owners["ktuner"])
        self.assertFalse(any("/components/" in path for path in owners["ktuner"]))

    def test_os_skills_native_rpm_preserves_legacy_skills_without_metadata(self) -> None:
        self.copy_source("os-skills", ())
        self.fixture("skills/example/SKILL.md")
        (self.source / "component.toml").unlink()
        owners = self.check_payload("os-skills")
        for prefix in ("usr/share/os-skills/skills", "usr/share/anolisa/skills"):
            self.assertIn(prefix + "/example/SKILL.md", owners["os-skills"])
        self.assertFalse(any("/components/" in path for path in owners["os-skills"]))

    def test_cosh_shell_and_skills_have_distinct_owners(self) -> None:
        self.copy_source("cosh-ng", ("packaging/systemd", "skills"))
        for binary in ("cosh-cli", "cosh-core", "cosh-shell", "cosh-gateway"):
            self.fixture("target/release/" + binary)
        owners = self.check_payload("cosh-ng")
        self.assertIn("usr/bin/cosh", owners["cosh-ng-core"])
        self.assertTrue(owners["cosh-ng-skills"])

    def test_sec_core_sandbox_and_native_hooks_own_their_compatibility_files(
        self,
    ) -> None:
        self.copy_source("agent-sec-core", ())
        legacy_files = [
            "usr/bin/agent-sec-cli",
            "usr/bin/agent-sec-daemon",
            "usr/bin/agent-sec-daemon-health",
            "usr/lib/systemd/system/agent-sec-core.service",
            "etc/agent-sec/skillsec.json",
            "usr/share/anolisa/components/sec-core/component.toml",
            "usr/local/bin/linux-sandbox",
            "usr/share/anolisa/extensions/agent-sec-core/hooks/code_scanner_hook.py",
            "opt/agent-sec/openclaw-plugin/scripts/deploy.sh",
            "opt/agent-sec/hermes-plugin/scripts/deploy.sh",
            "opt/agent-sec/codex-plugin/install.sh",
            "opt/agent-sec/qoder-plugin/install.sh",
            "opt/agent-sec/qwen-code-extension/scripts/deploy.sh",
            "opt/agent-sec/qwen-code-extension/hooks/observability_hook.py",
            "opt/agent-sec/qwen-code-extension/hooks/pii_checker_hook.py",
            "opt/agent-sec/qwen-code-extension/hooks/skill_ledger_hook.py",
            "usr/share/anolisa/skills/code-scanner/SKILL.md",
        ]
        prelude = "make() {\npython3 - <<'MAKE_FIXTURE'\nfrom pathlib import Path\n"
        prelude += "root = Path(" + repr(str(self.buildroot)) + ")\n"
        prelude += "for name in " + repr(legacy_files) + ":\n"
        prelude += "    target = root / name\n    target.parent.mkdir(parents=True, exist_ok=True)\n    target.write_text('fixture\\n')\n"
        prelude += "MAKE_FIXTURE\n}\n"
        owners = self.check_payload(
            "sec-core", "agent-sec-core.spec.v2.in", prelude, integration=True
        )
        self.assertIn("usr/bin/linux-sandbox", owners["agent-sec-sandbox"])
        self.assertNotIn("usr/bin/linux-sandbox", owners["agent-sec-cosh-hook"])
        self.assertIn(
            "usr/share/agent-sec-core/adapters/openclaw/scripts/deploy.sh",
            owners["agent-sec-openclaw-hook"],
        )
        self.assertIn(
            "opt/agent-sec/openclaw-plugin/scripts/deploy.sh",
            owners["agent-sec-openclaw-hook"],
        )


if __name__ == "__main__":
    unittest.main()
