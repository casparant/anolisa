#!/usr/bin/env python3
"""Exercise release installation with real Linux ANOLISA and AW executables."""

import argparse
import contextlib
import hashlib
import json
import os
import platform
import shutil
import subprocess
import tempfile
from pathlib import Path


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True) + "\n")


def command(
    arguments: list[str], environment: dict[str, str], success: bool = True
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(arguments, env=environment, text=True, capture_output=True, timeout=180)
    if (result.returncode == 0) != success:
        raise AssertionError(
            f"unexpected exit {result.returncode}: {arguments}\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )
    return result


def make_bundle(root: Path, anolisa: Path, aw: Path, commit: str) -> Path:
    """Use a two-package test composition; this does not build the full product."""
    bundle = root / "offline bundle"
    references = []
    for package_id, binary in [("aw-core", aw), ("anolisa-cli", anolisa)]:
        package = bundle / "packages" / package_id
        relative = "bin/aw" if package_id == "aw-core" else "bin/anolisa"
        payload = package / "payload" / relative
        payload.parent.mkdir(parents=True)
        shutil.copyfile(binary, payload)
        payload.chmod(0o755)
        owned = {relative: {"sha256": digest(payload), "mode": 0o755}}
        resource = (
            "share/aw/tests/release.json"
            if package_id == "aw-core"
            else "share/anolisa/tests/release.txt"
        )
        resource_path = package / "payload" / resource
        resource_path.parent.mkdir(parents=True)
        resource_path.write_text(
            '{"fixture":true}\n' if package_id == "aw-core" else "release resource\n"
        )
        resource_path.chmod(0o644)
        owned[resource] = {"sha256": digest(resource_path), "mode": 0o644}
        if package_id == "anolisa-cli":
            probe = package / "payload/bin/anolisa-resource-test"
            probe.write_text(
                "#!/bin/sh\nset -eu\n"
                'prefix=$(CDPATH= cd -P -- "$(dirname -- "$0")/.." && pwd -P)\n'
                'cat "$prefix/share/anolisa/tests/release.txt"\n'
                'printf "%s\\n" "${XDG_DATA_HOME:-$HOME/.local/share}"\n'
            )
            probe.chmod(0o755)
            owned["bin/anolisa-resource-test"] = {"sha256": digest(probe), "mode": 0o755}
        description = {
            "schema_version": 1,
            "package_id": package_id,
            "version": "1.0.0",
            "os": "linux",
            "arch": platform.machine(),
            "source_commit": commit,
            "files": owned,
            "dependencies": (
                [] if package_id == "aw-core" else [{"package_id": "aw-core", "version": "1.0.0"}]
            ),
        }
        write_json(package / "package.json", description)
        references.append(
            {
                "package_id": package_id,
                "version": "1.0.0",
                "path": f"packages/{package_id}",
                "description_sha256": digest(package / "package.json"),
            }
        )
    write_json(
        bundle / "release.json",
        {
            "schema_version": 1,
            "product_version": "1.0.0",
            "release_id": "linux-contract-" + commit[:12],
            "source_commit": commit,
            "os": "linux",
            "arch": platform.machine(),
            "abi": "gnu",
            "min_installer_version": "0.3.18",
            "packages": references,
            "entrypoints": {
                "anolisa": "bin/anolisa",
                "aw": "bin/aw",
                "anolisa-resource-test": "bin/anolisa-resource-test",
            },
        },
    )
    return bundle


def run(anolisa: Path, aw: Path, commit: str, cross_device: bool) -> None:
    if platform.system() != "Linux":
        raise SystemExit("the real AW installation test requires Linux")
    with contextlib.ExitStack() as stack:
        root = Path(
            stack.enter_context(
                tempfile.TemporaryDirectory(prefix="anolisa-release-", dir="/var/tmp")
            )
        )
        data = root / "data"
        if cross_device:
            data = Path(
                stack.enter_context(
                    tempfile.TemporaryDirectory(prefix="anolisa-data-", dir="/dev/shm")
                )
            )
            assert (
                data.stat().st_dev != root.stat().st_dev
            ), "cross-device test needs distinct filesystems"
        home = root / "home"
        home.mkdir(mode=0o700)
        state = root / "custom state"
        config = root / "custom config"
        environment = dict(os.environ)
        # These values are confined to the disposable test child processes.
        environment.update(
            {
                "HOME": str(home),
                "XDG_STATE_HOME": str(state),
                "XDG_CONFIG_HOME": str(config),
                "XDG_DATA_HOME": str(data),
                "XDG_CACHE_HOME": str(root / "cache"),
                "ANOLISA_UPDATE_URL": "http://127.0.0.1:1/must-not-fetch",
            }
        )
        bundle = make_bundle(root, anolisa, aw, commit)
        arguments = [
            str(anolisa),
            "release",
            "install",
            "--bundle",
            str(bundle),
            "--scope",
            "user",
            "--json",
        ]
        command(arguments + ["--dry-run"], environment)
        assert not (home / ".local").exists(), "preview created the release tree"
        assert not state.exists(), "preview created persistent state"

        command(arguments, environment)
        base = home / ".local/lib" / f"{platform.machine()}-linux-gnu/anolisa"
        prefix = base / "releases" / ("linux-contract-" + commit[:12])
        resources = (
            data
            / "anolisa/resources/releases"
            / f"{platform.machine()}-linux-gnu"
            / ("linux-contract-" + commit[:12])
        )
        assert prefix.is_dir() and not prefix.is_symlink()
        assert resources.is_dir() and not resources.is_symlink()
        assert (prefix / "share").is_symlink() and (prefix / "share").resolve() == resources
        assert (resources / "anolisa/tests/release.txt").read_text() == "release resource\n"
        assert (resources / "aw/tests/release.json").is_file()
        assert (base / "current").resolve() == prefix
        assert (prefix / ".aw-packages/aw-core.json").is_file()
        assert (state / "aw/packages").is_dir()
        assert digest(prefix / "bin/anolisa") == digest(anolisa)
        assert digest(prefix / "bin/aw") == digest(aw)
        for name in ["anolisa", "aw"]:
            command([str(home / ".local/bin" / name), "--version"], environment)
        probe = command([str(home / ".local/bin/anolisa-resource-test")], environment)
        assert probe.stdout.splitlines() == ["release resource", str(data)]
        inventory = json.loads(
            command(
                [str(aw), "list", "--installed", "--prefix", str(prefix), "--json"], environment
            ).stdout
        )
        installation = inventory["result"]["inventory"]["installations"][0]
        assert installation["resources"] == str(resources)

        managed = command(
            [str(prefix / "bin/anolisa"), "update", "self", "--dry-run", "--json"],
            environment,
            success=False,
        )
        assert "AW installation" in managed.stdout + managed.stderr
        assert not (prefix / "bin/anolisa.update-lock").exists()

        before = {name: (prefix / "bin" / name).stat().st_mtime_ns for name in ["anolisa", "aw"]}
        command(arguments, environment)
        assert before == {
            name: (prefix / "bin" / name).stat().st_mtime_ns for name in before
        }, "resume rewrote healthy payload"
        command([str(aw), "doctor", "--prefix", str(prefix), "--json"], environment)

        resource = resources / "anolisa/tests/release.txt"
        resource.write_text("edited resource\n")
        command([str(aw), "doctor", "--prefix", str(prefix), "--json"], environment, success=False)
        command(arguments, environment, success=False)
        assert resource.read_text() == "edited resource\n"
        resource.write_text("release resource\n")

        installed_cli = prefix / "bin/anolisa"
        original = installed_cli.read_bytes()
        installed_cli.write_bytes(original + b"user-edit")
        command(arguments, environment, success=False)
        assert installed_cli.read_bytes() == original + b"user-edit"
        installed_cli.write_bytes(original)

        source = bundle / "packages/anolisa-cli/payload/bin/anolisa"
        source.write_bytes(b"tampered source")
        command(arguments, environment, success=False)
        assert digest(installed_cli) == digest(anolisa)
        assert (base / "current").resolve() == prefix
        command(
            [
                str(aw),
                "uninstall",
                "anolisa-cli",
                "--prefix",
                str(prefix),
                "--scope",
                "user",
                "--yes",
                "--json",
            ],
            environment,
        )
        assert not installed_cli.exists()
        assert not resource.exists(), "uninstall did not remove mapped resources"
        assert (
            resources / "aw/tests/release.json"
        ).is_file(), "uninstall removed another package's resource"
        print(
            "real Linux AW: preview, split program/resources, custom XDG, launchers, resume, "
            "doctor, self-update protection, tamper refusal, and mapped uninstall passed"
        )


def system_install(anolisa: Path, aw: Path, commit: str, reader_uid: int) -> None:
    """Use only on a disposable root CI runner with empty product roots."""
    if platform.system() != "Linux" or os.geteuid() != 0 or reader_uid == 0:
        raise SystemExit("system smoke requires Linux root and a non-root reader UID")
    if any(
        path.exists()
        for path in [
            Path("/usr/local/lib/anolisa"),
            Path("/usr/local/share/anolisa"),
            Path("/var/lib/aw/packages"),
        ]
    ):
        raise SystemExit("system smoke requires empty product and AW database roots")
    with tempfile.TemporaryDirectory(prefix="anolisa-system-", dir="/var/tmp") as temporary:
        root = Path(temporary)
        bundle = make_bundle(root, anolisa, aw, commit)
        environment = dict(os.environ)
        arguments = [
            str(anolisa),
            "release",
            "install",
            "--bundle",
            str(bundle),
            "--scope",
            "system",
            "--json",
        ]
        command(arguments, environment)
        prefix = Path("/usr/local/lib/anolisa/releases") / ("linux-contract-" + commit[:12])
        resources = Path("/usr/local/share/anolisa/releases") / ("linux-contract-" + commit[:12])
        assert Path("/usr/local/lib/anolisa/current").resolve() == prefix
        assert (prefix / "share").resolve() == resources
        assert (resources / "aw/tests/release.json").is_file()
        assert (prefix / "bin/aw").stat().st_uid == 0
        reader = ["sudo", "-u", f"#{reader_uid}", "--"]
        command(reader + [str(prefix / "bin/anolisa"), "--version"], environment)
        command(reader + ["/usr/local/bin/anolisa", "--version"], environment)
        template = (
            Path("/usr/local/etc/anolisa/releases")
            / f"{platform.machine()}-linux-gnu"
            / ("linux-contract-" + commit[:12] + ".providers.json")
        )
        generated = json.loads(command(reader + ["/bin/cat", str(template)], environment).stdout)
        assert generated["kind"] == "anolisa-provider-template"
        assert generated["runtime_context"]["scope"] == "invoking-user"
        assert "/root/" not in json.dumps(generated)
        probe = command(
            reader
            + [
                "env",
                "XDG_DATA_HOME=/var/tmp/anolisa-reader-data",
                "/usr/local/bin/anolisa-resource-test",
            ],
            environment,
        )
        assert probe.stdout.splitlines() == ["release resource", "/var/tmp/anolisa-reader-data"]
        command(
            reader
            + [
                str(prefix / "bin/aw"),
                "list",
                "--installed",
                "--prefix",
                str(prefix),
                "--scope",
                "system",
                "--json",
            ],
            environment,
        )
        command(
            reader
            + [
                str(prefix / "bin/aw"),
                "doctor",
                "--prefix",
                str(prefix),
                "--scope",
                "system",
                "--json",
            ],
            environment,
        )
        assert Path("/var/lib/aw/packages").is_dir()
        print(
            "real Linux system AW: root installation and non-root execution, "
            "inventory, and doctor passed"
        )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--anolisa", type=Path, required=True)
    parser.add_argument("--aw", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument(
        "--system", action="store_true", help="root-only smoke on a disposable CI runner"
    )
    parser.add_argument("--reader-uid", type=int)
    parser.add_argument(
        "--cross-device",
        action="store_true",
        help="place user resources on a distinct tmpfs filesystem",
    )
    options = parser.parse_args()
    if options.system:
        if options.reader_uid is None:
            parser.error("--system requires --reader-uid")
        system_install(
            options.anolisa.resolve(),
            options.aw.resolve(),
            options.source_commit,
            options.reader_uid,
        )
    else:
        run(
            options.anolisa.resolve(),
            options.aw.resolve(),
            options.source_commit,
            options.cross_device,
        )
