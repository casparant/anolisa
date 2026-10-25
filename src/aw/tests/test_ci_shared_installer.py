"""The AW gate must execute the extracted engine and distributor tooling checks."""

import json
from pathlib import Path
import tomllib
from unittest.mock import patch

from test_ci_checks import AW, GateFixture, gate


class SharedInstallerGateTests(GateFixture):
    def test_preview_workflow_checks_out_and_caches_shared_path_dependency(self) -> None:
        repo = AW.parents[1]
        preview = repo / "distribution/tooling/aw-preview"
        manifest = tomllib.loads((preview / "Cargo.toml").read_text())
        dependency = manifest["workspace"]["dependencies"]["local-package"]["path"]
        engine = (preview / dependency).resolve().relative_to(repo).as_posix()
        workflow = (repo / ".github/workflows/aw-preview.yml").read_text()
        plan = workflow.split("  plan:\n", 1)[1].split("  packages:\n", 1)[0]
        packages = workflow.split("  packages:\n", 1)[1].split("  native:\n", 1)[0]
        for section, key in ((plan, "sparse-checkout"), (packages, "workspaces")):
            with self.subTest(key=key):
                paths = (
                    section.split(f"          {key}: |\n", 1)[1]
                    .split("      - ", 1)[0]
                    .splitlines()
                )
                self.assertIn(engine, [path.strip() for path in paths])

    def test_shared_workspaces_have_full_checks_and_compiled_inventory(self) -> None:
        repo = self.root
        aw = repo / "src/aw"
        engine = repo / "distribution/crates/local-package"
        preview = repo / "distribution/tooling/aw-preview"
        providers = [repo / "providers" / name for name in ("agent-sec-core", "tokenless")]
        calls = []

        def run(command: list[str], cwd: Path, **kwargs: object) -> str:
            calls.append((command, cwd))
            if command[:2] == ["cargo", "metadata"]:
                return json.dumps(
                    {"workspace_root": str(cwd), "packages": [], "workspace_members": []}
                )
            return ""

        with patch.dict(gate.os.environ), patch.object(gate, "REPO", repo), patch.object(
            gate, "AW", aw
        ), patch.object(gate, "candidate", return_value="a" * 40), patch.object(
            gate, "selftest"
        ), patch.object(
            gate, "run", side_effect=run
        ), patch.object(
            gate, "structure"
        ), patch.object(
            gate, "package_permissions"
        ), patch.object(
            gate, "check_inventory"
        ) as inventory, patch(
            "check_native.load"
        ), patch(
            "check_legacy_cli.prepare_legacy_cli", return_value=repo / "old-installer"
        ):
            gate.check()
        for root in [aw, *providers, engine, preview]:
            for command in ("fmt", "clippy", "test", "doc"):
                self.assertTrue(
                    any(args[:2] == ["cargo", command] and cwd == root for args, cwd in calls),
                    f"missing {command} for {root}",
                )
        shared = inventory.call_args.args[0]["shared_workspaces"]
        self.assertEqual([row["workspace_root"] for row in shared], [str(engine), str(preview)])
