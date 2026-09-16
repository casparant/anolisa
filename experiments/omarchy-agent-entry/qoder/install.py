#!/usr/bin/env python3
"""Repackage the official Qoder DEB for pacman; never execute vendor install scripts."""

from __future__ import annotations

import argparse
import configparser
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parent


def read_config(path: Path) -> tuple[str, str]:
    config = configparser.ConfigParser(interpolation=None)
    with path.open(encoding="utf-8") as handle:
        config.read_file(handle)
    url = config.get("qoder", "url").strip()
    parsed = urlsplit(url)
    if (
        parsed.scheme != "https"
        or parsed.hostname not in ("download.qoder.com", "download.qoder.com.cn")
        or parsed.username
        or parsed.password
        or parsed.port not in (None, 443)
        or parsed.fragment
    ):
        raise ValueError("Use an HTTPS download.qoder.com or download.qoder.com.cn URL in conf.")
    digest = config.get("qoder", "sha256", fallback="").strip().lower()
    if digest and not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise ValueError("conf sha256 must be empty or a 64-character SHA256 digest.")
    return url, digest


def parse_control(control: str) -> tuple[str, str]:
    fields = {}
    for line in control.splitlines():
        if line and not line[0].isspace() and ":" in line:
            key, value = line.split(":", 1)
            if key in fields:
                raise ValueError("Duplicate DEB control field: " + key)
            fields[key] = value.strip()
    if fields.get("Package") != "qoder" or fields.get("Architecture") != "amd64":
        raise ValueError("Expected the official qoder GUI package for amd64, not IDE/CLI/ARM.")
    match = re.fullmatch(r"(?:(\d+):)?([0-9][A-Za-z0-9.+~_-]*)", fields.get("Version", ""))
    if not match:
        raise ValueError("Unsupported DEB version: " + fields.get("Version", "<missing>"))
    # Arch keeps the epoch separate and disallows hyphens inside pkgver.
    return str(int(match[1] or "0")), match[2].replace("-", "_")


def package_metadata(deb: Path, work: Path) -> tuple[str, str]:
    control_archive = work / "control.tar.xz"
    with control_archive.open("wb") as handle:
        subprocess.run(["bsdtar", "-xOf", str(deb), "control.tar.xz"], stdout=handle, check=True)
    control = subprocess.check_output(
        ["bsdtar", "-xOf", str(control_archive), "./control"], text=True
    )
    return parse_control(control)


def sha256(path: Path) -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def require_host(build_only: bool) -> None:
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise ValueError("This converter targets Linux x86_64 with pacman, not macOS or ARM64.")
    if os.geteuid() == 0:
        raise ValueError("Run as your normal desktop user, not sudo. Only pacman needs sudo.")
    for tool in ("makepkg", "pacman", "bsdtar", "curl", "zstd", "bash"):
        if not shutil.which(tool):
            raise ValueError(
                "Missing " + tool + ". Install prerequisites: "
                "sudo pacman -S --needed base-devel curl libarchive zstd python"
            )
    if not build_only:
        for tool in ("sudo", "unshare"):
            if not shutil.which(tool):
                raise ValueError("Missing required tool: " + tool)
        probe = subprocess.run(["unshare", "--user", "true"], capture_output=True, text=True)
        if probe.returncode:
            raise ValueError(
                "Unprivileged user namespaces are unavailable: "
                + probe.stderr.strip()
                + ". Qoder needs an Electron sandbox. This converter will not disable it, "
                "enable setuid, or change security policy; ask your system administrator."
            )


def build(deb: Path, work: Path, digest: str) -> tuple[Path, str]:
    epoch, version = package_metadata(deb, work)
    shutil.copy2(ROOT / "PKGBUILD", work / "PKGBUILD")
    build_env = os.environ.copy()
    build_env.update(
        QODER_VERSION=version,
        QODER_EPOCH=epoch,
        QODER_SHA256=digest,
        PKGDEST=str(work),
        SRCDEST=str(work),
    )
    print(f"Repackaging Qoder {epoch}:{version}; no application source compilation.", flush=True)
    # Repacking does not need GUI libraries. pacman -U resolves runtime dependencies.
    subprocess.run(["makepkg", "--nodeps"], cwd=work, env=build_env, check=True)
    paths = subprocess.check_output(
        ["makepkg", "--packagelist"], cwd=work, env=build_env, text=True
    ).splitlines()
    if len(paths) != 1:
        raise ValueError("Expected exactly one Arch package from makepkg.")
    package = Path(paths[0]).resolve()
    if not package.is_file() or package.parent != work.resolve():
        raise ValueError("makepkg did not produce its package in the private build directory.")
    return package, f"{epoch}:{version}"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--conf", type=Path, default=ROOT / "qoder.conf")
    parser.add_argument("--deb", type=Path, help="Use an already downloaded official DEB")
    parser.add_argument(
        "--build-only", action="store_true", help="Build without sudo or installation"
    )
    args = parser.parse_args(argv)
    try:
        url, expected = read_config(args.conf)
        require_host(args.build_only)
        if args.deb and not args.deb.is_file():
            raise ValueError("DEB does not exist: " + str(args.deb))
        print("Source: " + (str(args.deb.resolve()) if args.deb else url), flush=True)
        if not args.build_only:
            print("Build a local anolisa-qoder-bin package, then install with sudo pacman -U.")
            print(
                "This installs proprietary Qoder and dependencies; existing files are not forced."
            )
            if not sys.stdin.isatty() or input("Type INSTALL to continue: ").strip() != "INSTALL":
                raise ValueError("Cancelled; no package downloaded or installed.")
        cache = Path(os.environ.get("XDG_CACHE_HOME", str(Path.home() / ".cache")))
        cache = (cache / "anolisa-agent-entry/qoder").resolve()
        cache.mkdir(parents=True, exist_ok=True)
        work = Path(tempfile.mkdtemp(prefix="build-", dir=cache))
        print("Download / build cache (kept for diagnosis): " + str(work), flush=True)
        deb = work / "Qoder.deb"
        if args.deb:
            shutil.copyfile(args.deb, deb)
        else:
            subprocess.run(
                [
                    "curl",
                    "--fail",
                    "--location",
                    "--show-error",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--connect-timeout",
                    "30",
                    "--max-time",
                    "1800",
                    "--output",
                    str(deb),
                    url,
                ],
                check=True,
            )
        digest = sha256(deb)
        if expected and digest != expected:
            raise ValueError("SHA256 mismatch; refusing to build or install.")
        print("SHA256: " + digest + (" (matched conf pin)" if expected else " (local record only)"))
        package, version = build(deb, work, digest)
        (work / "receipt.json").write_text(
            json.dumps(
                {
                    "source": str(args.deb) if args.deb else url,
                    "version": version,
                    "sha256": digest,
                    "package": str(package),
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        print("Package: " + str(package), flush=True)
        if args.build_only:
            print(
                "Build complete. Install on a suitable desktop with: sudo pacman -U " + str(package)
            )
            return 0
        subprocess.run(["sudo", "pacman", "-U", str(package)], check=True)
        print("Installed. Refresh the Dashboard or run: qoder-desktop")
        print("Update: rerun this script. Remove: sudo pacman -R anolisa-qoder-bin")
        print("Browser login handlers: qoder.desktop (qoder: and qoder-app:).")
        print("If another app owns a handler, see the README before changing the default.")
        return 0
    except (OSError, ValueError, EOFError, configparser.Error, subprocess.SubprocessError) as error:
        print("Qoder installation stopped: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
