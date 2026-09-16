import importlib.util
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("installer", ROOT / "install.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.target = self.home / ".config/omarchy/plugins/anolisa.agent-entry"
        self.hook = self.home / ".config/omarchy/hooks/post-boot.d/anolisa-agent-entry.sh"
        for mock in (
            patch.object(installer.Path, "home", return_value=self.home),
            patch.object(installer.sys, "platform", "linux"),
            patch.object(installer.os, "geteuid", return_value=1000),
            patch.object(installer.shutil, "which", return_value="/usr/bin/mock"),
            patch.object(installer, "run"),
            patch.object(installer.subprocess, "run", return_value=SimpleNamespace(returncode=0)),
        ):
            mock.start()
            self.addCleanup(mock.stop)

    def invoke(self, *args: str) -> None:
        with patch.object(installer.sys, "argv", ["install.py", *args]):
            installer.main()

    def test_install_and_recoverable_uninstall(self) -> None:
        other_hook = self.hook.parent / "unrelated.sh"
        other_hook.parent.mkdir(parents=True)
        other_hook.write_text("unchanged")
        config = self.home / ".config/anolisa/agent-entry.json"
        config.parent.mkdir()
        config.write_text('{"version":1,"agents":[]}')
        self.invoke("--autostart")
        self.assertTrue((self.target / "Dashboard.qml").is_file())
        self.assertTrue((self.home / ".local/bin/anolisa-agent-entry").is_symlink())
        self.assertIn(" open ", self.hook.read_text())
        self.invoke("--uninstall")
        self.assertFalse(self.target.exists())
        self.assertFalse(self.hook.exists())
        self.assertEqual(other_hook.read_text(), "unchanged")
        self.assertTrue(config.exists())
        self.assertTrue(list((self.home / ".config/anolisa/plugin-backups").glob("removed-*")))

    def test_upgrade_backs_up_plugin_and_hook(self) -> None:
        self.invoke("--autostart")
        (self.target / "local-note.txt").write_text("keep this")
        self.invoke("--replace", "--autostart")
        backups = list((self.home / ".config/anolisa/plugin-backups").glob("upgrade-*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(
            (backups[0] / "plugins-anolisa.agent-entry/local-note.txt").read_text(), "keep this"
        )
        self.assertTrue(self.hook.exists())


if __name__ == "__main__":
    unittest.main()
