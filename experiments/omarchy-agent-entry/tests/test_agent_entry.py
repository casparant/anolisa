import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("agent_entry", ROOT / "agent_entry.py")
entry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(entry)


class EntryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.cfg = Path(self.temp.name) / "agent-entry.json"
        self.patcher = patch.object(entry, "config_path", return_value=self.cfg)
        self.patcher.start()
        self.addCleanup(self.patcher.stop)

    def test_status_does_not_spawn_or_read_credentials(self) -> None:
        with patch.object(entry, "which", return_value=None), patch.object(
            entry.subprocess, "Popen"
        ) as spawn:
            result = entry.snapshot()
            self.assertGreaterEqual(len(result["agents"]), 5)
            self.assertFalse(any(a["available"] for a in result["agents"]))
            spawn.assert_not_called()

    def test_argument_injection_is_not_shell(self) -> None:
        args = type(
            "Args",
            (),
            dict(
                id="example",
                name="Example",
                kind="terminal",
                value='["echo", "$(touch /tmp/never)"]',
            ),
        )()
        entry.register(args)
        self.assertEqual(entry.get_agent("local.example")["argv"][1], "$(touch /tmp/never)")
        with patch.object(entry.subprocess, "Popen") as spawn:
            entry.detach(["echo", "$(touch /tmp/never)"])
            self.assertNotIn("shell", spawn.call_args.kwargs)
            self.assertTrue(spawn.call_args.kwargs["start_new_session"])

    def test_register_web_and_duplicate(self) -> None:
        args = type(
            "Args",
            (),
            dict(id="console", name="Console", kind="web", value="http://127.0.0.1:19820"),
        )()
        entry.register(args)
        self.assertEqual(entry.get_agent("local.console")["kind"], "web")
        with self.assertRaises(ValueError):
            entry.register(args)

    def test_invalid_web_and_commands(self) -> None:
        for url in ("file:///etc/passwd", "javascript:alert(1)", "https://user:pass@example.com"):
            with self.assertRaises(ValueError):
                entry.valid_url(url)
        for argv in ([], "echo hi", ["echo", "bad\narg"], [1]):
            with self.assertRaises(ValueError):
                entry.valid_argv(argv)

    def test_bad_config_is_not_overwritten(self) -> None:
        self.cfg.write_text("not JSON")
        with self.assertRaises(ValueError):
            entry.load_config()
        self.assertEqual(self.cfg.read_text(), "not JSON")

    def test_project_requires_directory(self) -> None:
        with self.assertRaises(ValueError):
            entry.project_dir(str(Path(self.temp.name) / "absent"))
        self.assertEqual(entry.project_dir(self.temp.name), Path(self.temp.name).resolve())

    def test_gui_does_not_resolve_cli_name(self) -> None:
        item = entry.get_agent("qoder")
        with patch.object(
            entry, "which", side_effect=lambda s: "/bin/qoder" if s == "qoder" else None
        ):
            self.assertIsNone(entry.command_for(item))

    def test_unknown_id(self) -> None:
        with self.assertRaises(ValueError):
            entry.launch("not-an-agent", self.temp.name)

    def test_terminal_payload_is_argument_array(self) -> None:
        with patch.object(
            entry, "which", side_effect=lambda s: "/usr/bin/foot" if s == "foot" else None
        ):
            argv = entry.terminal_argv(["echo", "a b; c"], Path("/tmp/a b"))
            self.assertEqual(argv[-1], "a b; c")
            self.assertIn("--working-directory=/tmp/a b", argv)

    def test_installer_requires_confirmation(self) -> None:
        with patch.object(entry.sys.stdin, "isatty", return_value=False):
            with self.assertRaises(ValueError):
                entry.confirm("test")

    def test_manifest(self) -> None:
        manifest = json.loads((ROOT / "manifest.json").read_text())
        self.assertEqual(manifest["id"], entry.PLUGIN_ID)
        self.assertIn("panel", manifest["kinds"])
        for file in manifest["entryPoints"].values():
            self.assertTrue((ROOT / file).is_file())
            self.assertNotIn("..", file)


if __name__ == "__main__":
    unittest.main()
