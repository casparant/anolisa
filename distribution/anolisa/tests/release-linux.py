#!/usr/bin/env python3
"""Exercise product-owned Linux installation, AW isolation and ordinary-user runtime use."""

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


def profile_selection(anolisa: Path, aw: Path, commit: str) -> None:
    """Use real file transactions to switch a schema-two product's public command set."""
    with tempfile.TemporaryDirectory(prefix="anolisa-profile-", dir="/var/tmp") as temporary:
        root = Path(temporary)
        home = root / "home"
        home.mkdir(mode=0o700)
        environment = dict(
            os.environ,
            HOME=str(home),
            XDG_DATA_HOME=str(root / "data"),
            XDG_STATE_HOME=str(root / "state"),
            XDG_CONFIG_HOME=str(root / "config"),
            XDG_CACHE_HOME=str(root / "cache"),
        )
        bundle = make_bundle(root, anolisa, aw, commit)
        package = bundle / "packages/example-feature"
        program = package / "payload/bin/anolisa-feature-test"
        program.parent.mkdir(parents=True)
        program.write_text("#!/bin/sh\nprintf 'feature works\\n'\n")
        program.chmod(0o755)
        write_json(
            package / "package.json",
            {
                "schema_version": 1,
                "package_id": "example-feature",
                "version": "1.0.0",
                "os": "linux",
                "arch": platform.machine(),
                "source_commit": commit,
                "files": {"bin/anolisa-feature-test": {"sha256": digest(program), "mode": 0o755}},
                "dependencies": [{"package_id": "aw-core", "version": "1.0.0"}],
            },
        )
        manifest = json.loads((bundle / "release.json").read_text())
        manifest["schema_version"] = 2
        manifest["min_installer_version"] = "0.3.19"
        manifest["packages"].append(
            {
                "package_id": "example-feature",
                "version": "1.0.0",
                "path": "packages/example-feature",
                "description_sha256": digest(package / "package.json"),
            }
        )
        manifest["entrypoints"]["anolisa-feature-test"] = "bin/anolisa-feature-test"
        manifest["distribution"] = {
            "schema_version": 1,
            "default_profiles": {"user": "minimal", "system": "minimal"},
            "profiles": {
                "minimal": {
                    "packages": ["aw-core", "anolisa-cli"],
                    "scopes": ["user", "system"],
                    "preset": "disabled",
                },
                "with-feature": {
                    "packages": ["aw-core", "anolisa-cli", "example-feature"],
                    "scopes": ["user"],
                    "preset": "disabled",
                },
            },
            "components": {"example-feature": ["example-feature"]},
            "presets": {"disabled": {"services": []}},
        }
        write_json(bundle / "release.json", manifest)
        arguments = [str(anolisa), "install", "--bundle", str(bundle), "--scope", "user", "--json"]
        feature = json.loads(
            command(arguments + ["--profile", "with-feature"], environment).stdout
        )["data"]
        launcher = home / ".local/bin/anolisa-feature-test"
        assert command([str(launcher)], environment).stdout == "feature works\n"
        minimal = json.loads(command(arguments + ["--profile", "minimal"], environment).stdout)[
            "data"
        ]
        assert (
            feature["prefix"] != minimal["prefix"]
        ), "different selections shared a resumable prefix"
        assert not launcher.exists(), "omitted package retained a broken public command"
        assert not (Path(minimal["prefix"]) / "bin/anolisa-feature-test").exists()
        command(arguments + ["--profile", "minimal"], environment)
        command([str(home / ".local/bin/aw"), "--version"], environment)
        print("real Linux AW: schema-two profile selection and omitted command cleanup passed")


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
        assert (state / "anolisa/packages").is_dir()
        assert not (state / "aw/packages").exists()
        assert digest(prefix / "bin/anolisa") == digest(anolisa)
        assert digest(prefix / "bin/aw") == digest(aw)
        for name in ["anolisa", "aw"]:
            command([str(home / ".local/bin" / name), "--version"], environment)
        probe = command([str(home / ".local/bin/anolisa-resource-test")], environment)
        assert probe.stdout.splitlines() == ["release resource", str(data)]
        inventory = json.loads(
            command(
                [str(anolisa), "release", "list", "--scope", "user", "--json"], environment
            ).stdout
        )
        installation = inventory["data"]["installations"][0]
        assert installation["resources"] == str(resources)

        managed = command(
            [str(prefix / "bin/anolisa"), "update", "self", "--dry-run", "--json"],
            environment,
            success=False,
        )
        assert "managed installation" in managed.stdout + managed.stderr
        assert not (prefix / "bin/anolisa.update-lock").exists()

        before = {name: (prefix / "bin" / name).stat().st_mtime_ns for name in ["anolisa", "aw"]}
        command(arguments, environment)
        assert before == {
            name: (prefix / "bin" / name).stat().st_mtime_ns for name in before
        }, "resume rewrote healthy payload"
        command([str(anolisa), "release", "doctor", "--bundle", str(bundle), "--json"], environment)

        resource = resources / "anolisa/tests/release.txt"
        resource.write_text("edited resource\n")
        command([str(anolisa), "release", "doctor", "--bundle", str(bundle), "--json"], environment, success=False)
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
        # AW must not acquire ownership of a product managed by ANOLISA.
        command([str(aw), "list", "--installed", "--prefix", str(prefix), "--json"], environment, success=False)
        command([str(aw), "uninstall", "anolisa-cli", "--prefix", str(prefix), "--yes", "--json"], environment, success=False)
        assert installed_cli.is_file() and resource.is_file()
        assert not (state / "aw/packages").exists()
        # Whole-release cleanup requires switching away from that immutable build.
        shutil.copyfile(anolisa, source)
        source.chmod(0o755)
        next_bundle = make_bundle(root / "next", anolisa, aw, commit)
        next_manifest = json.loads((next_bundle / "release.json").read_text())
        next_manifest["release_id"] += "-next"
        write_json(next_bundle / "release.json", next_manifest)
        command([str(anolisa), "release", "install", "--bundle", str(next_bundle), "--json"], environment)
        removal = [str(anolisa), "release", "uninstall", "--bundle", str(bundle), "--json"]
        command(removal, environment, success=False)
        command(removal + ["--dry-run"], environment)
        assert installed_cli.is_file(), "removal preview deleted package files"
        command(removal + ["--yes"], environment)
        command(removal + ["--yes"], environment)
        assert not installed_cli.exists() and not resource.exists()
        assert not (resources / "aw/tests/release.json").exists()
        assert (prefix / ".aw-packages/aw-core.json").is_file(), "empty registration was discarded"
        command([str(home / ".local/bin/anolisa"), "--version"], environment)
        command([str(anolisa), "release", "recover", "--scope", "user", "--json"], environment)
        print("Linux product installation: private package database, cross-device resources, launchers, "
              "resume, verification, AW ownership isolation and inactive cleanup passed")



def system_install(anolisa: Path, aw: Path, commit: str, reader_uid: int) -> None:
    """Use only on a disposable root CI runner with empty product roots."""
    if platform.system() != "Linux" or os.geteuid() != 0 or reader_uid == 0:
        raise SystemExit("system smoke requires Linux root and a non-root reader UID")
    if any(
        path.exists()
        for path in [
            Path("/usr/local/lib/anolisa"),
            Path("/usr/local/share/anolisa"),
            Path("/var/lib/anolisa/packages"),
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
        command([str(anolisa), "release", "list", "--scope", "system", "--json"], environment)
        command([str(anolisa), "release", "doctor", "--bundle", str(bundle), "--scope", "system", "--json"], environment)
        command(reader + [str(anolisa), "release", "list", "--scope", "system", "--json"], environment, success=False)
        command(reader + [str(prefix / "bin/aw"), "list", "--installed", "--prefix", str(prefix), "--scope", "system", "--json"], environment, success=False)
        assert Path("/var/lib/anolisa/packages").is_dir()
        assert Path("/var/lib/anolisa").stat().st_mode & 0o777 == 0o700
        assert not Path("/var/lib/aw/packages").exists()
        print("Linux system product: root installation, private package records and ordinary-user execution passed")


def without_aw_installer(anolisa: Path, aw: Path, commit: str) -> None:
    """A bundled AW trap and an AW-free composition must both install successfully."""
    with tempfile.TemporaryDirectory(prefix="anolisa-no-aw-", dir="/var/tmp") as temporary:
        root = Path(temporary)
        home = root / "home"
        home.mkdir(mode=0o700)
        marker = root / "aw-must-not-run"
        environment = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(root / "state"),
                           XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"),
                           XDG_CACHE_HOME=str(root / "cache"), ANOLISA_FORBIDDEN_AW_CALL=str(marker))
        bundle = make_bundle(root, anolisa, aw, commit)
        payload = bundle / "packages/aw-core/payload/bin/aw"
        payload.write_text('#!/bin/sh\nprintf invoked > "$ANOLISA_FORBIDDEN_AW_CALL"\nexit 99\n')
        payload.chmod(0o755)
        descriptor = bundle / "packages/aw-core/package.json"
        package = json.loads(descriptor.read_text())
        package["files"]["bin/aw"]["sha256"] = digest(payload)
        write_json(descriptor, package)
        manifest = json.loads((bundle / "release.json").read_text())
        manifest["packages"][0]["description_sha256"] = digest(descriptor)
        write_json(bundle / "release.json", manifest)
        command([str(anolisa), "release", "install", "--bundle", str(bundle), "--json"], environment)
        assert not marker.exists(), "product installer invoked the AW runtime"
        # Reuse the product CLI payload without declaring or delivering AW at all.
        manifest["release_id"] += "-cli-only"
        manifest["min_installer_version"] = "0.3.19"
        manifest["packages"] = [item for item in manifest["packages"] if item["package_id"] != "aw-core"]
        manifest["entrypoints"].pop("aw")
        descriptor = bundle / "packages/anolisa-cli/package.json"
        package = json.loads(descriptor.read_text())
        package["dependencies"] = []
        write_json(descriptor, package)
        manifest["packages"][0]["description_sha256"] = digest(descriptor)
        shutil.rmtree(bundle / "packages/aw-core")
        write_json(bundle / "release.json", manifest)
        command([str(anolisa), "release", "install", "--bundle", str(bundle), "--json"], environment)
        command([str(home / ".local/bin/anolisa"), "--version"], environment)
        assert not (home / ".local/bin/aw").exists()
        assert not marker.exists()
        assert not (root / "state/aw/packages").exists()
        print("Linux product: AW trap never invoked and AW-free composition installed")



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
        profile_selection(options.anolisa.resolve(), options.aw.resolve(), options.source_commit)
        without_aw_installer(options.anolisa.resolve(), options.aw.resolve(), options.source_commit)
