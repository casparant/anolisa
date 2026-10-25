#!/usr/bin/env python3
"""Exercise bootstrap downloads against a real loopback HTTP server."""

from __future__ import annotations

import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

INSTALLER = Path(__file__).resolve().parents[1] / "static/install.sh"


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        self.server.requests.append(self.path)
        body = self.server.routes.get(self.path)
        if body is None:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args: object) -> None:
        pass


def archive(entries: list[tuple[str, bytes, int, bytes]]) -> bytes:
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz", format=tarfile.USTAR_FORMAT) as tar:
        for name, content, mode, kind in entries:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.mode = mode
            member.size = len(content) if kind == tarfile.REGTYPE else 0
            if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                member.linkname = "../../outside"
            tar.addfile(member, io.BytesIO(content) if member.isfile() else None)
    return output.getvalue()


class BootstrapTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="anolisa-bootstrap-")
        self.root = Path(self.temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        (self.root / "home").mkdir()
        (self.bin / "python3").symlink_to(sys.executable)
        self.executable(
            "uname",
            '#!/bin/bash\ncase "$1" in -s) echo "${TEST_OS:-Linux}";; -m) echo "${TEST_ARCH:-x86_64}";; esac\n',
        )
        self.executable("id", "#!/bin/bash\necho 1000\n")
        self.executable("sudo", '#!/bin/bash\necho called > "$TEST_SUDO"\nexit 99\n')
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.routes = {}
        self.server.requests = []
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.log = self.root / "commands"

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        self.temporary.cleanup()

    def executable(self, name: str, content: str) -> Path:
        path = self.bin / name
        path.write_text(content)
        path.chmod(0o755)
        return path

    def run_installer(
        self, args: list[str], env: dict[str, str] | None = None
    ) -> subprocess.CompletedProcess[str]:
        environment = {
            key: value for key, value in os.environ.items() if not key.startswith("ANOLISA_")
        }
        environment.update(
            {
                "PATH": f"{self.bin}:{os.environ['PATH']}",
                "HOME": str(self.root / "home"),
                "TEST_LOG": str(self.log),
                "TEST_SUDO": str(self.root / "sudo"),
            }
        )
        environment.update(env or {})
        return subprocess.run(
            ["/bin/bash", str(INSTALLER), *args],
            env=environment,
            text=True,
            capture_output=True,
            timeout=35,
            cwd=self.root,
        )

    def product(
        self,
        extra: list[tuple[str, bytes, int, bytes]] | None = None,
        exit_code: int = 0,
        schema_version: int = 1,
    ) -> dict[str, object]:
        root = "anolisa-1.2.3-linux-x86_64-gnu"
        launcher = b'#!/bin/bash\nset -eu\nroot="$(cd "$(dirname "$0")" && pwd)"\nexec "$root/packages/anolisa-cli/payload/bin/anolisa" release install --bundle "$root" "$@"\n'
        cli = f'#!/bin/bash\nprintf "%s\\0" "$@" > "$TEST_LOG"\nexit {exit_code}\n'.encode()
        entries = [
            (root, b"", 0o755, tarfile.DIRTYPE),
            (root + "/install.sh", launcher, 0o755, tarfile.REGTYPE),
            (
                root + "/release.json",
                json.dumps({"schema_version": schema_version, "product_version": "1.2.3"}).encode(),
                0o644,
                tarfile.REGTYPE,
            ),
            (root + "/packages/anolisa-cli/payload/bin/anolisa", cli, 0o755, tarfile.REGTYPE),
        ]
        data = archive(entries + (extra or []))
        self.server.routes["/product.tar.gz"] = data
        index = {
            "schema_version": 1,
            "product_version": "1.2.3",
            "artifacts": [
                {
                    "os": "linux",
                    "arch": "x86_64",
                    "abi": "gnu",
                    "url": self.url + "/product.tar.gz",
                    "sha256": hashlib.sha256(data).hexdigest(),
                    "size": len(data),
                }
            ],
        }
        self.server.routes["/index.json"] = json.dumps(index).encode()
        return index

    def legacy(self) -> dict[str, str]:
        cli = b'#!/bin/bash\nif [ "${1:-}" = --version ]; then echo "anolisa 1.2.3"; else printf "%s\\n" "$*" >> "$TEST_LOG"; fi\n'
        data = archive([("anolisa", cli, 0o755, tarfile.REGTYPE)])
        for os_name, target in [
            ("linux", "x86_64-unknown-linux-gnu"),
            ("macos", "x86_64-apple-darwin"),
        ]:
            path = f"/mirror/anolisa-releases/anolisa/v1/cli/releases/1.2.3/artifacts/{os_name}/x86_64/anolisa-cli-1.2.3-{target}.tar.gz"
            self.server.routes[path] = data
            self.server.routes[path + ".sha256.txt"] = (
                hashlib.sha256(data).hexdigest().encode() + b"  archive\n"
            )
        return {
            "ANOLISA_VERSION": "1.2.3",
            "ANOLISA_MIRROR": self.url + "/mirror",
            "ANOLISA_INSTALL_DIR": str(self.root / "installed-cli"),
        }

    def test_product_index_archive_and_local_entrypoint_share_one_verified_route(self) -> None:
        self.product()
        result = self.run_installer(
            [
                "--release-url",
                self.url + "/index.json",
                "--release-version",
                "1.2.3",
                "--scope",
                "user",
            ]
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        arguments = self.log.read_bytes().split(b"\0")[:-1]
        self.assertEqual(arguments[:3], [b"release", b"install", b"--bundle"])
        self.assertEqual(arguments[-2:], [b"--scope", b"user"])
        self.assertIn(b"anolisa-1.2.3-linux-x86_64-gnu", arguments[3])
        self.assertEqual(self.server.requests, ["/index.json", "/product.tar.gz"])
        self.assertFalse((self.root / "home/.local/bin/anolisa").exists())
        self.assertFalse((self.root / "sudo").exists())

    def test_product_environment_variables_and_failed_install_preserve_exit_without_fallback(
        self,
    ) -> None:
        self.product(exit_code=23)
        result = self.run_installer(
            [],
            {"ANOLISA_RELEASE_URL": self.url + "/index.json", "ANOLISA_RELEASE_VERSION": "1.2.3"},
        )
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertTrue(self.log.exists())
        self.assertEqual(self.server.requests, ["/index.json", "/product.tar.gz"])
        self.assertFalse((self.root / "home/.local/bin/anolisa").exists())

    def test_schema_two_profile_is_forwarded_to_the_verified_bundled_installer(self) -> None:
        self.product(schema_version=2)
        result = self.run_installer(
            [
                "--release-url",
                self.url + "/index.json",
                "--release-version",
                "1.2.3",
                "--scope",
                "user",
                "--profile",
                "standard-user",
            ]
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        arguments = self.log.read_bytes().split(b"\0")[:-1]
        self.assertEqual(arguments[-4:], [b"--scope", b"user", b"--profile", b"standard-user"])
        self.assertFalse((self.root / "sudo").exists())

    def test_profile_environment_uses_product_protocol_and_rejects_legacy_mixing(self) -> None:
        self.product(schema_version=2)
        environment = {
            "ANOLISA_RELEASE_PROFILE": "minimal",
            "ANOLISA_RELEASE_VERSION": "1.2.3",
            "ANOLISA_RELEASE_URL": self.url + "/index.json",
        }
        result = self.run_installer([], environment)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_bytes().split(b"\0")[-3:-1], [b"--profile", b"minimal"])
        self.server.requests.clear()
        result = self.run_installer(["--component", "tokenless"], environment)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot be combined", result.stderr)
        self.assertEqual(self.server.requests, [])

    def test_all_legacy_overrides_conflict_with_product_inputs_before_network(self) -> None:
        for key in [
            "ANOLISA_VERSION",
            "ANOLISA_UPDATE_URL",
            "ANOLISA_INSTALL_DIR",
            "ANOLISA_MIRROR",
        ]:
            with self.subTest(key=key):
                result = self.run_installer(["--release-url", self.url + "/index.json"], {key: ""})
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("cannot be combined", result.stderr)
        for arguments in [
            ["--component", "cosh-ng"],
            ["--cosh-ng"],
            ["--component", "cosh-ng", "--backend", "rpm"],
            ["--component", "cosh-ng", "--upgrade"],
            ["--component", "cosh-ng", "--uninstall"],
            ["--component", "cosh-ng", "--install-mode", "user"],
            ["--cli-only"],
        ]:
            with self.subTest(arguments=arguments):
                result = self.run_installer(arguments + ["--scope", "user"])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("cannot be combined", result.stderr)
        self.assertEqual(self.server.requests, [])

    def test_legacy_flags_keep_old_cli_archive_and_argument_semantics_over_real_http(self) -> None:
        environment = self.legacy()
        environment["ANOLISA_UPDATE_URL"] = self.url + "/unused-pinned-update-url"
        for options, expected in [
            ([], ""),
            (
                ["--component", "cosh-ng", "--backend", "raw", "--install-mode", "user"],
                "--install-mode user install cosh-ng --backend raw\n",
            ),
            (
                ["--component", "cosh-ng", "--install-mode", "user", "--upgrade"],
                "--install-mode user update cosh-ng\n",
            ),
            (
                ["--component", "cosh-ng", "--install-mode", "user", "--uninstall"],
                "--install-mode user uninstall cosh-ng\n",
            ),
        ]:
            with self.subTest(options=options):
                self.log.unlink(missing_ok=True)
                result = self.run_installer(options, environment)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.log.read_text() if self.log.exists() else "", expected)
        self.assertTrue((self.root / "installed-cli/anolisa").is_file())
        self.assertTrue(all("/v1/cli/releases/1.2.3/" in path for path in self.server.requests))
        self.assertNotIn("/unused-pinned-update-url", self.server.requests)

    def test_macos_remains_on_the_legacy_cli_route(self) -> None:
        environment = self.legacy()
        environment["TEST_OS"] = "Darwin"
        result = self.run_installer(["--cli-only"], environment)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("/artifacts/macos/x86_64/", self.server.requests[0])
        self.assertFalse(self.log.exists())

    def test_tampered_checksum_size_version_and_duplicate_json_never_execute(self) -> None:
        for damage in ["checksum", "size", "version", "duplicate", "oversized"]:
            with self.subTest(damage=damage):
                index = self.product()
                if damage == "checksum":
                    index["artifacts"][0]["sha256"] = "0" * 64
                elif damage == "size":
                    index["artifacts"][0]["size"] += 1
                elif damage == "version":
                    index["product_version"] = "2.0.0"
                elif damage == "oversized":
                    index["artifacts"][0]["size"] = 4 * 1024**3 + 1
                body = json.dumps(index).encode()
                if damage == "duplicate":
                    body = body.replace(
                        b'"schema_version": 1', b'"schema_version": 1, "schema_version": 1'
                    )
                self.server.routes["/index.json"] = body
                result = self.run_installer(
                    ["--release-url", self.url + "/index.json", "--release-version", "1.2.3"]
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.log.exists())

    def test_traversal_links_duplicate_paths_and_dangerous_modes_never_extract_or_execute(
        self,
    ) -> None:
        root = "anolisa-1.2.3-linux-x86_64-gnu/"
        for extra in [
            ("../outside", b"bad", 0o644, tarfile.REGTYPE),
            ("/outside", b"bad", 0o644, tarfile.REGTYPE),
            (root + "link", b"", 0o755, tarfile.SYMTYPE),
            (root + "hardlink", b"", 0o755, tarfile.LNKTYPE),
            (root + "install.sh", b"evil", 0o755, tarfile.REGTYPE),
            (root + "suid", b"evil", 0o4755, tarfile.REGTYPE),
        ]:
            with self.subTest(extra=extra[0]):
                self.product([extra])
                result = self.run_installer(["--release-url", self.url + "/index.json"])
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.log.exists())
                self.assertFalse((self.root / "outside").exists())

    def test_gnu_long_names_from_the_product_builder_are_accepted(self) -> None:
        index = self.product()
        output = io.BytesIO()
        original = self.server.routes["/product.tar.gz"]
        with tarfile.open(fileobj=io.BytesIO(original), mode="r:gz") as source:
            with tarfile.open(fileobj=output, mode="w:gz", format=tarfile.GNU_FORMAT) as target:
                for member in source:
                    target.addfile(member, source.extractfile(member) if member.isfile() else None)
                member = tarfile.TarInfo("anolisa-1.2.3-linux-x86_64-gnu/share/" + "long" * 40)
                member.mode = 0o644
                target.addfile(member, io.BytesIO(b""))
        data = output.getvalue()
        self.server.routes["/product.tar.gz"] = data
        index["artifacts"][0].update(sha256=hashlib.sha256(data).hexdigest(), size=len(data))
        self.server.routes["/index.json"] = json.dumps(index).encode()
        result = self.run_installer(["--release-url", self.url + "/index.json"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.log.exists())

    def test_pax_sparse_regular_files_are_rejected_before_execution(self) -> None:
        index = self.product()
        output = io.BytesIO()
        original = self.server.routes["/product.tar.gz"]
        with tarfile.open(fileobj=io.BytesIO(original), mode="r:gz") as source:
            with tarfile.open(fileobj=output, mode="w:gz", format=tarfile.PAX_FORMAT) as target:
                for member in source:
                    target.addfile(member, source.extractfile(member) if member.isfile() else None)
                member = tarfile.TarInfo("anolisa-1.2.3-linux-x86_64-gnu/sparse")
                member.mode = 0o644
                member.size = 1
                member.pax_headers = {"GNU.sparse.map": "0,1", "GNU.sparse.size": "1"}
                target.addfile(member, io.BytesIO(b"x"))
        data = output.getvalue()
        self.server.routes["/product.tar.gz"] = data
        index["artifacts"][0].update(sha256=hashlib.sha256(data).hexdigest(), size=len(data))
        self.server.routes["/index.json"] = json.dumps(index).encode()
        result = self.run_installer(["--release-url", self.url + "/index.json"])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("sparse files", result.stderr)
        self.assertFalse(self.log.exists())

    def test_aw_native_and_symlinked_owned_clis_are_rejected_before_download(self) -> None:
        environment = self.legacy()
        prefix = self.root / "managed"
        (prefix / "bin").mkdir(parents=True)
        (prefix / ".aw-packages").mkdir()
        binary = prefix / "bin/anolisa"
        binary.write_bytes(b"existing managed CLI")
        for destination in [prefix / "bin", self.root / "alias-bin"]:
            destination.mkdir(exist_ok=True)
            if destination != prefix / "bin":
                (destination / "anolisa").symlink_to(binary)
            environment["ANOLISA_INSTALL_DIR"] = str(destination)
            result = self.run_installer([], environment)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("AW-managed", result.stderr)
        environment["ANOLISA_INSTALL_DIR"] = str(self.root / "native-bin")
        Path(environment["ANOLISA_INSTALL_DIR"]).mkdir()
        native = Path(environment["ANOLISA_INSTALL_DIR"]) / "anolisa"
        native.write_bytes(b"existing native CLI")
        for manager in ["rpm", "dpkg-query"]:
            with self.subTest(manager=manager):
                fake = self.executable(manager, "#!/bin/bash\nexit 0\n")
                result = self.run_installer([], environment)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("owned CLI", result.stderr)
                self.assertEqual(native.read_bytes(), b"existing native CLI")
                fake.unlink()
        self.assertEqual(binary.read_bytes(), b"existing managed CLI")
        self.assertEqual(self.server.requests, [])

    def test_product_launcher_is_never_overwritten_by_cli_or_rpm_bootstrap(self) -> None:
        environment = self.legacy()
        directory = Path(environment["ANOLISA_INSTALL_DIR"])
        directory.mkdir()
        launcher = directory / "anolisa"
        contents = b'#!/bin/sh\n# ANOLISA release launcher v1\nexec /nonexistent/anolisa "$@"\n'
        launcher.write_bytes(contents)
        launcher.chmod(0o755)
        for arguments in (["--cli-only"], ["--rpm"]):
            with self.subTest(arguments=arguments):
                result = self.run_installer(arguments, environment)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("ANOLISA product launcher", result.stderr)
                self.assertEqual(launcher.read_bytes(), contents)
        self.assertEqual(self.server.requests, [])

    def test_missing_cli_under_a_symlinked_managed_prefix_is_protected(self) -> None:
        environment = self.legacy()
        prefix = self.root / "managed"
        (prefix / "bin").mkdir(parents=True)
        (prefix / ".aw-install.lock").touch()
        directory = self.root / "bin-alias"
        directory.symlink_to(prefix / "bin")
        environment["ANOLISA_INSTALL_DIR"] = str(directory)
        result = self.run_installer([], environment)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("AW-managed", result.stderr)
        self.assertFalse((prefix / "bin/anolisa").exists())
        self.assertEqual(self.server.requests, [])

    def test_missing_native_owned_cli_is_not_recreated_by_the_legacy_installer(self) -> None:
        environment = self.legacy()
        self.executable("rpm", "#!/bin/bash\nexit 0\n")
        result = self.run_installer([], environment)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("RPM-owned CLI", result.stderr)
        self.assertFalse(Path(environment["ANOLISA_INSTALL_DIR"]).exists())
        self.assertEqual(self.server.requests, [])

    def test_linux_default_reports_python_requirement_without_starting_network(self) -> None:
        tools = self.root / "minimal"
        tools.mkdir()
        (tools / "uname").symlink_to(self.bin / "uname")
        for tool in ["mktemp", "rm"]:
            (tools / tool).symlink_to(shutil.which(tool))
        result = self.run_installer([], {"PATH": str(tools)})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Python 3 is required", result.stderr)
        self.assertEqual(self.server.requests, [])


if __name__ == "__main__":
    unittest.main()
