"""Exercise the OCI exec entrypoint without a Docker engine or real daemons."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ENTRYPOINT = Path(__file__).resolve().parents[1] / "entrypoint.sh"


class EntrypointTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.runtime = self.root / "runtime"
        self.runtime.mkdir(mode=0o700)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.log = self.root / "argv.json"
        # Emulate GNU stat's one used operation using the actual inode facts;
        # this keeps the security checks exercised on macOS as well as Linux.
        self.executable("stat", """import os, stat, sys
assert sys.argv[1:4] == ['-c', '%u:%a', '--']
entry = os.stat(sys.argv[4], follow_symlinks=False)
print(f'{entry.st_uid}:{stat.S_IMODE(entry.st_mode):o}')
""")
        self.executable("anolisa", """import json, os, sys
with open(os.environ['ANOLISA_ENTRYPOINT_TEST_LOG'], 'w') as output:
    json.dump(sys.argv[1:], output)
""")
        self.environment = dict(os.environ)
        self.environment.update({
            "PATH": str(self.bin) + os.pathsep + os.environ.get("PATH", ""),
            "XDG_RUNTIME_DIR": str(self.runtime),
            "ANOLISA_ENTRYPOINT_TEST_LOG": str(self.log),
        })

    def executable(self, name: str, source: str) -> None:
        path = self.bin / name
        path.write_text(f"#!{sys.executable}\n{source}", encoding="utf-8")
        path.chmod(0o755)

    def run_entrypoint(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["/bin/sh", str(ENTRYPOINT), *arguments],
            env=self.environment,
            text=True,
            capture_output=True,
            timeout=5,
            check=False,
        )

    def test_shell_syntax_and_exec_preserve_literal_arguments(self) -> None:
        subprocess.run(["/bin/sh", "-n", str(ENTRYPOINT)], check=True)
        arguments = ["serve", "--preset", "container", "--config", "a file $HOME.json"]
        result = self.run_entrypoint(*arguments)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(self.log.read_text()), arguments)

    def test_relative_runtime_is_rejected_before_execution(self) -> None:
        self.environment["XDG_RUNTIME_DIR"] = "relative"
        self.assertEqual(self.run_entrypoint("serve").returncode, 2)
        self.assertFalse(self.log.exists())

    def test_shared_or_symlinked_runtime_is_rejected_before_execution(self) -> None:
        self.runtime.chmod(0o755)
        self.assertEqual(self.run_entrypoint("serve").returncode, 2)
        self.runtime.chmod(0o700)
        link = self.root / "runtime-link"
        link.symlink_to(self.runtime, target_is_directory=True)
        self.environment["XDG_RUNTIME_DIR"] = str(link)
        self.assertEqual(self.run_entrypoint("serve").returncode, 2)
        self.assertFalse(self.log.exists())

if __name__ == "__main__":
    unittest.main()
