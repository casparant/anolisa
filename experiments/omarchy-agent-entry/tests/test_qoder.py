import importlib.util
import io
import os
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("qoder_installer", ROOT / "qoder/install.py")
qoder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qoder)


def tar_bytes(files: dict[str, bytes]) -> bytes:
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w:xz") as archive:
        for name, content in files.items():
            member = tarfile.TarInfo(name)
            member.mode = 0o755 if name.startswith("./opt/") else 0o644
            member.size = len(content)
            archive.addfile(member, io.BytesIO(content))
    return stream.getvalue()


def write_deb(path: Path, control: str) -> None:
    data = tar_bytes(
        {
            "./opt/Qoder/qoder": b"fixture, not an executable",
            "./opt/Qoder/chrome-sandbox": b"fixture",
            "./usr/share/applications/qoder.desktop": (
                b"[Desktop Entry]\nName=Qoder\nExec=/opt/Qoder/qoder %U\n"
                b"MimeType=x-scheme-handler/qoder;x-scheme-handler/qoder-app;\n"
            ),
            "./usr/share/icons/hicolor/16x16/apps/qoder.png": b"fixture",
        }
    )
    with path.open("wb") as handle:
        handle.write(b"!<arch>\n")
        for name, body in (
            ("debian-binary", b"2.0\n"),
            ("control.tar.xz", tar_bytes({"./control": control.encode()})),
            ("data.tar.xz", data),
        ):
            header = f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(body):<10}`\n"
            handle.write(header.encode() + body)
            if len(body) % 2:
                handle.write(b"\n")


class QoderTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.control = "Package: qoder\nArchitecture: amd64\nVersion: 1:0.2.5\n"

    def test_config_is_data_not_shell(self) -> None:
        url, digest = qoder.read_config(ROOT / "qoder/qoder.conf")
        self.assertTrue(url.startswith("https://download.qoder.com/"))
        self.assertEqual(digest, "")
        conf = self.work / "test.conf"
        for url in (
            "http://download.qoder.com/pkg.deb",
            "https://download.qoder.com.evil.example/pkg.deb",
            "https://user:pass@download.qoder.com/pkg.deb",
            "$(touch /tmp/never)",
        ):
            conf.write_text("[qoder]\nurl = " + url)
            with self.assertRaises(ValueError):
                qoder.read_config(conf)
        conf.write_text("[qoder]\nurl=https://download.qoder.com/pkg.deb\nsha256=bad")
        with self.assertRaises(ValueError):
            qoder.read_config(conf)

    def test_version_epoch_and_revision(self) -> None:
        self.assertEqual(qoder.parse_control(self.control), ("1", "0.2.5"))
        self.assertEqual(
            qoder.parse_control(self.control.replace("1:0.2.5", "0.3.1-2")), ("0", "0.3.1_2")
        )

    def test_reject_wrong_package_arch_and_version(self) -> None:
        for bad in (
            self.control.replace("qoder", "qoder-ide"),
            self.control.replace("amd64", "arm64"),
            self.control.replace("1:0.2.5", "$(touch /tmp/never)"),
            self.control + "Package: injected\n",
        ):
            with self.assertRaises(ValueError):
                qoder.parse_control(bad)

    def test_reject_host_and_root(self) -> None:
        with patch.object(qoder.platform, "system", return_value="Darwin"):
            with self.assertRaises(ValueError):
                qoder.require_host(False)
        with patch.object(qoder.platform, "system", return_value="Linux"), patch.object(
            qoder.platform, "machine", return_value="x86_64"
        ), patch.object(qoder.os, "geteuid", return_value=0):
            with self.assertRaises(ValueError):
                qoder.require_host(False)

    def test_cancel_before_download(self) -> None:
        with patch.object(qoder, "require_host"), patch.object(
            qoder.sys.stdin, "isatty", return_value=True
        ), patch("builtins.input", return_value="no"), patch.object(qoder, "build") as build:
            self.assertEqual(qoder.main([]), 1)
            build.assert_not_called()

    def test_bad_digest_never_builds(self) -> None:
        conf = self.work / "pin.conf"
        conf.write_text("[qoder]\nurl=https://download.qoder.com/pkg.deb\nsha256=" + "a" * 64)
        deb = self.work / "Qoder.deb"
        deb.write_bytes(b"not matching")
        with patch.object(qoder, "require_host"), patch.dict(
            os.environ, {"XDG_CACHE_HOME": str(self.work)}
        ), patch.object(qoder, "build") as build:
            self.assertEqual(
                qoder.main(["--build-only", "--conf", str(conf), "--deb", str(deb)]), 1
            )
            build.assert_not_called()

    def test_build_only_never_installs(self) -> None:
        deb = self.work / "Qoder.deb"
        deb.write_bytes(b"fixture")
        with patch.object(qoder, "require_host"), patch.dict(
            os.environ, {"XDG_CACHE_HOME": str(self.work)}
        ), patch.object(
            qoder, "build", return_value=(self.work / "qoder.pkg.tar.zst", "1:0.2.5")
        ), patch.object(
            qoder.subprocess, "run"
        ) as run:
            self.assertEqual(qoder.main(["--build-only", "--deb", str(deb)]), 0)
            run.assert_not_called()

    @unittest.skipUnless(shutil.which("bsdtar") and shutil.which("bash"), "Requires libarchive")
    def test_real_package_layout_from_small_deb(self) -> None:
        source = self.work / "src"
        package = self.work / "pkg"
        source.mkdir()
        package.mkdir()
        deb = source / "Qoder.deb"
        write_deb(deb, self.control)
        self.assertEqual(qoder.package_metadata(deb, self.work), ("1", "0.2.5"))
        environment = os.environ.copy()
        environment.update(
            srcdir=str(source),
            pkgdir=str(package),
            QODER_VERSION="0.2.5",
            QODER_EPOCH="1",
            QODER_SHA256=qoder.sha256(deb),
        )
        subprocess.run(
            ["bash", "-ec", 'source "$1"; package', "fixture-test", str(ROOT / "qoder/PKGBUILD")],
            env=environment,
            check=True,
        )
        desktop = (package / "usr/share/applications/qoder.desktop").read_text()
        self.assertIn("Exec=/usr/bin/qoder-desktop %U", desktop)
        self.assertIn("x-scheme-handler/qoder-app;", desktop)
        wrapper = package / "usr/bin/qoder-desktop"
        self.assertIn('exec /opt/Qoder/qoder "$@"', wrapper.read_text())
        self.assertTrue(wrapper.stat().st_mode & 0o111)
        self.assertFalse((package / "usr/bin/qoder").exists())
        self.assertEqual((package / "opt/Qoder/chrome-sandbox").stat().st_mode & 0o7777, 0o755)
        self.assertFalse((package / "DEBIAN").exists())
        self.assertTrue((package / "usr/share/icons/hicolor/16x16/apps/qoder.png").is_file())


if __name__ == "__main__":
    unittest.main()
