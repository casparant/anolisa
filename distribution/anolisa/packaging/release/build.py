#!/usr/bin/env python3
"""Build the complete Linux product through existing component release builders.

Source builds require Linux x86_64, a clean checkout, Python 3.11+, Git, Bash,
Rust 1.93.0 and AW's pinned 1.97.1, cross 0.2.5, Docker with the repository's
verified gnu2.17/gnu2.28 x86_64 images, cargo-sbom, Node 22.23.2/npm, make, just,
uvx, patch, GNU tar/gzip, readelf, sha256sum, flock, install, cc, curl and realpath. Existing component
builders retain their own toolchain, ABI, dependency and release verification.
--staged-input never invokes these builders and does not imply a source build.
"""

from __future__ import annotations

import argparse
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import Any

from assemble import assemble
from release_common import (
    COMMIT,
    composition,
    digest,
    files,
    json_bytes,
    read_json,
    require,
    version,
)
from stage import cargo_package, copy_file, raw_package, unpack_raw, write_description

BASE = Path(__file__).resolve().parent
SOURCE_DEFAULT = BASE.parents[3]
ACTION_ENV = {
    "anolisa": "ANOLISA_SOURCE_WORKTREE",
    "cosh-ng": "COSH_NG_SOURCE_WORKTREE",
    "tokenless": "TOKENLESS_SOURCE_WORKTREE",
    "sec-core": "SEC_CORE_SOURCE_WORKTREE",
    "agentsight": "AGENTSIGHT_SOURCE_WORKTREE",
    "agent-memory": "AGENT_MEMORY_SOURCE_WORKTREE",
    "ws-ckpt": "WS_CKPT_SOURCE_WORKTREE",
    "skillfs": "SKILLFS_SOURCE_WORKTREE",
    "os-skills": "OS_SKILLS_SOURCE_WORKTREE",
}


def git(source: Path, *arguments: str) -> str:
    """Read or operate on one explicitly selected source checkout."""
    return subprocess.check_output(["git", "-C", str(source), *arguments], text=True).strip()


def clean_source(source: Path, ignored: list[Path]) -> str:
    """Pin HEAD only when no unpublished source would be silently excluded."""
    result = subprocess.run(
        ["git", "-C", str(source), "diff", "--quiet", "HEAD", "--"], check=False
    )
    require(
        result.returncode == 0,
        "source checkout has unpublished tracked changes; commit them before a source build",
    )
    untracked = (
        subprocess.check_output(
            ["git", "-C", str(source), "ls-files", "--others", "--exclude-standard", "-z"]
        )
        .decode()
        .split("\0")
    )
    for name in filter(None, untracked):
        path = (source / name).absolute()
        require(
            any(path.is_relative_to(root) for root in ignored),
            f"source checkout has unpublished input: {name}",
        )
    return git(source, "rev-parse", "HEAD")


def source_versions(source: Path, specs: list[dict[str, Any]]) -> dict[str, str]:
    """Read component-owned versions, never replace them with the product version."""
    result: dict[str, str] = {}
    for spec in specs:
        with (source / spec["version_file"]).open("rb") as stream:
            value: Any = tomllib.load(stream)
        for key in spec["version_key"].split("."):
            value = value[key]
        result[spec["package_id"]] = version(value)
    return result


def run(command: list[str], cwd: Path, env: dict[str, str], log: Path) -> None:
    """Retain each builder's complete diagnostics in the isolated work directory."""
    print(f"Building {log.stem}; log: {log}", flush=True)
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("wb") as stream:
        result = subprocess.run(command, cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT)
    require(result.returncode == 0, f"builder exited {result.returncode}; see {log}")


def preflight() -> None:
    """Fail early instead of returning a platform tarball missing a component."""
    require(
        platform.system() == "Linux" and platform.machine() == "x86_64",
        "complete source builds require Linux x86_64; sec-core's pinned raw runtime "
        "currently supports x86_64 only; use explicit complete staged inputs otherwise",
    )
    require(platform.libc_ver()[0] == "glibc", "source build host must use GNU libc")
    commands = [
        "bash",
        "git",
        "cargo",
        "rustup",
        "cross",
        "docker",
        "cargo-sbom",
        "node",
        "npm",
        "make",
        "just",
        "uvx",
        "patch",
        "tar",
        "gzip",
        "readelf",
        "sha256sum",
        "flock",
        "install",
        "cc",
        "curl",
        "realpath",
    ]
    missing = [command for command in commands if shutil.which(command) is None]
    require(not missing, "missing source-build tools: " + ", ".join(missing))
    installed = subprocess.check_output(["rustup", "toolchain", "list"], text=True)
    for toolchain in ("1.93.0", "1.97.1"):
        require(
            any(line.startswith(toolchain + "-") for line in installed.splitlines()),
            f"missing source-build Rust toolchain: {toolchain}",
        )
    require(
        subprocess.check_output(["node", "--version"], text=True).strip() == "v22.23.2",
        "complete release builders require Node 22.23.2",
    )


def build_action(
    source: Path,
    work: Path,
    spec: dict[str, Any],
    component_version: str,
    arch: str,
    env: dict[str, str],
) -> Path:
    """Reuse the component's existing reproducible build and raw packaging checks."""
    builder = spec["builder"]
    output = work / "artifacts" / builder
    action_env = env.copy()
    action_env[ACTION_ENV[builder]] = str(work / "worktrees" / builder)
    profile = spec["profile"]
    if profile != "static-linux":
        profile += "-" + arch
    command = [
        "bash",
        str(source / ".github/actions" / f"build-{builder}-prebuilt/build.sh"),
        "--source-repo",
        str(source),
        "--output-dir",
        str(output),
        "--version",
        component_version,
        "--target-os",
        "linux",
        "--target-arch",
        arch,
        "--profile",
        profile,
    ]
    if builder == "tokenless":
        command += ["--wheel-output-dir", str(work / "artifacts/tokenless-wheels")]
    run(command, source, action_env, work / "logs" / (builder + ".log"))
    archive = output / f"{builder}-{component_version}-linux-{arch}.tar.gz"
    require(archive.is_file(), f"component builder did not produce expected archive: {archive}")
    return archive


def build_cargo(source: Path, work: Path, spec: dict[str, Any], env: dict[str, str]) -> Path:
    """Compile unbundled AW/framework bridges without invoking its Preview composer."""
    package_id = spec["package_id"]
    directory = source / spec["source"]
    target = work / "cargo" / package_id
    toolchain = "1.97.1" if package_id != "ktuner" else "1.93.0"
    command = [
        "cargo",
        "+" + toolchain,
        "build",
        "--release",
        "--locked",
        "--manifest-path",
        str(directory / "Cargo.toml"),
        "--bin",
        spec["binary"],
        "--target-dir",
        str(target),
    ]
    native_env = env.copy()
    native_env["RUSTUP_TOOLCHAIN"] = toolchain
    command = [
        sys.executable,
        str(source / ".github/actions/prebuilt-rust-common/reproducible-build.py"),
        "--source-root",
        str(source),
        "--source-date-epoch",
        env["SOURCE_DATE_EPOCH"],
        "--",
        *command,
    ]
    run(command, directory, native_env, work / "logs" / (package_id + ".log"))
    return target / "release" / spec["binary"]


def native_sbom(
    source: Path,
    work: Path,
    spec: dict[str, Any],
    description: Path,
    component_version: str,
    arch: str,
    env: dict[str, str],
) -> None:
    """Reuse the release SBOM generator, binding SDK dependencies to package metadata."""
    package_id = spec["package_id"]
    artifact = work / "artifacts" / package_id / (package_id + "-package.json")
    copy_file(description, artifact, False)
    checksum = artifact.with_name(artifact.name + ".sha256")
    checksum.write_text(f"{digest(artifact)}  {artifact.name}\n", encoding="utf-8")
    native_env = env.copy()
    native_env["RUSTUP_TOOLCHAIN"] = "1.93.0" if package_id == "ktuner" else "1.97.1"
    command = [
        sys.executable,
        str(source / ".github/actions/prebuilt-rust-common/generate-sbom.py"),
        "--artifact",
        str(artifact),
        "--component",
        package_id,
        "--version",
        component_version,
        "--os",
        "linux",
        "--arch",
        arch,
        "--target",
        arch + "-unknown-linux-gnu",
        "--project-dir",
        str(source / spec["source"]),
        "--source-date-epoch",
        env["SOURCE_DATE_EPOCH"],
    ]
    run(command, source / spec["source"], native_env, work / "logs" / (package_id + "-sbom.log"))


def rtk_provenance(
    source: Path, work: Path, payload: Path, source_commit: str, env: dict[str, str]
) -> None:
    """Carry the pinned RTK license and patch inputs, using its existing setup entrypoint."""
    root = work / "rtk-inputs"
    shared = root / "crates/tokenless-stats/src"
    shared.mkdir(parents=True)
    for name in ("home.rs", "path_policy.rs"):
        copy_file(source / "src/tokenless/crates/tokenless-stats/src" / name, shared / name, False)
    rtk = root / "third_party/rtk"
    run(
        ["bash", str(source / "src/tokenless/scripts/setup-rtk.sh"), str(rtk)],
        source,
        env,
        work / "logs/rtk-provenance.log",
    )
    evidence: list[dict[str, str]] = []
    documents = [
        ("LICENSE", rtk / "LICENSE"),
        ("source-commit", rtk / ".anolisa-rtk-commit"),
        ("tokenless-stats/home.rs", shared / "home.rs"),
        ("tokenless-stats/path_policy.rs", shared / "path_policy.rs"),
    ]
    documents += [
        ("patches/" + name, source / "src/tokenless/third_party/patches" / name)
        for name in ("rtk-tokenless-stats.patch", "rtk-pytest-error-report.patch")
    ]
    documents.append(("setup-rtk.sh", source / "src/tokenless/scripts/setup-rtk.sh"))
    destination = payload / "share/doc/tokenless/rtk"
    for name, origin in documents:
        copy_file(origin, destination / name, False)
        evidence.append({"path": name, "sha256": digest(origin)})
    (destination / "build-inputs.json").write_bytes(
        json_bytes(
            {
                "anolisa_source_commit": source_commit,
                "rtk_source_commit": (rtk / ".anolisa-rtk-commit").read_text().strip(),
                "inputs": evidence,
            }
        )
    )
    (destination / "build-inputs.json").chmod(0o644)


def elf_dependencies(staged: Path) -> list[dict[str, Any]]:
    """Record actual GNU dynamic dependencies without claiming a shared glibc baseline."""
    result: list[dict[str, Any]] = []
    for package in sorted(staged.iterdir()):
        for name, path in files(package / "payload").items():
            with path.open("rb") as stream:
                if stream.read(4) != b"\x7fELF":
                    continue
            dynamic = subprocess.check_output(
                ["readelf", "--dynamic", "--wide", str(path)], text=True
            )
            versions = subprocess.check_output(
                ["readelf", "--version-info", "--wide", str(path)], text=True
            )
            headers = subprocess.check_output(
                ["readelf", "--program-headers", "--wide", str(path)], text=True
            )
            result.append(
                {
                    "package_id": package.name,
                    "path": name,
                    "sha256": digest(path),
                    "needed": sorted(set(re.findall(r"Shared library: \[([^]]+)\]", dynamic))),
                    "version_requirements": sorted(
                        set(re.findall(r"Name: ([A-Za-z][A-Za-z0-9_.]+)", versions))
                    ),
                    "interpreter": re.findall(
                        r"Requesting program interpreter: ([^]]+)\]", headers
                    ),
                }
            )
    return result


def source_build(args: argparse.Namespace) -> Path:
    """Build and stage every component from one immutable clean checkout."""
    preflight()
    require(args.arch == "x86_64", "complete source builds currently support x86_64 only")
    source = Path(git(args.source_repo, "rev-parse", "--show-toplevel")).absolute()
    output = args.output_dir.absolute()
    ignored = [output]
    if args.work_dir:
        ignored.append(args.work_dir.absolute())
    source_commit = clean_source(source, ignored)
    if args.source_commit:
        require(args.source_commit == source_commit, "source-commit differs from checkout HEAD")
    epoch = int(git(source, "show", "-s", "--format=%ct", source_commit))
    require(args.epoch is None or args.epoch == epoch, "source build epoch must match the commit")
    require(not output.exists(), f"refusing existing output: {output}")
    if args.work_dir:
        work = args.work_dir.absolute()
        require(not work.exists(), f"work-dir must be new: {work}")
        work.mkdir(parents=True, mode=0o700)
    else:
        work = Path(tempfile.mkdtemp(prefix="anolisa-release-build-"))
    print(f"Isolated source build directory: {work}", flush=True)
    snapshot = work / "source"
    subprocess.run(
        ["git", "-C", str(source), "worktree", "add", "--detach", str(snapshot), source_commit],
        check=True,
    )
    try:
        specs = composition(BASE / "linux.json")
        versions = source_versions(snapshot, specs)
        product_version = args.product_version or versions["anolisa-cli"]
        release_version = product_version.lower().replace("+", "_")
        release_id = args.release_id or f"{release_version}-{source_commit[:12]}"
        env = os.environ.copy()
        env.pop("CARGO_TARGET_DIR", None)
        env.pop("CARGO_BUILD_TARGET", None)
        env.pop("RUSTUP_TOOLCHAIN", None)
        env.pop("RUSTFLAGS", None)
        env.pop("CARGO_ENCODED_RUSTFLAGS", None)
        env["SOURCE_DATE_EPOCH"] = str(epoch)
        env["LC_ALL"] = "C"
        for profile in ("gnu2.17-x86_64", "gnu2.28-x86_64"):
            run(
                [
                    "bash",
                    str(snapshot / ".github/actions/prebuilt-rust-common/cross-profile.sh"),
                    profile,
                    "--check",
                ],
                snapshot,
                env,
                work / "logs" / (profile + "-preflight.log"),
            )
        staged = work / "packages"
        staged.mkdir()
        build_inputs: list[dict[str, Any]] = []
        for spec in specs:
            package_id = spec["package_id"]
            target = staged / package_id
            if spec["builder"] == "cargo":
                binary = build_cargo(snapshot, work, spec, env)
                cargo_package(binary, target, spec, snapshot)
            else:
                archive = build_action(snapshot, work, spec, versions[package_id], args.arch, env)
                raw = work / "raw" / package_id
                unpack_raw(archive, raw)
                raw_package(raw, target, spec, snapshot)
            if package_id == "tokenless":
                rtk_provenance(snapshot, work, target / "payload", source_commit, env)
            write_description(target, spec, versions, args.arch, source_commit)
            if spec["builder"] == "cargo":
                native_sbom(
                    snapshot,
                    work,
                    spec,
                    target / "package.json",
                    versions[package_id],
                    args.arch,
                    env,
                )
            build_inputs.append(
                {
                    "package_id": package_id,
                    "version": versions[package_id],
                    "builder": spec["builder"],
                    "profile": spec.get("profile", "native-gnu"),
                }
            )
        require(
            clean_source(source, ignored) == source_commit,
            "source checkout changed during the release build",
        )
        provenance_files = work / "provenance"
        provenance_files.mkdir()
        for directory in sorted((work / "artifacts").iterdir()):
            if directory.name == "tokenless-wheels":
                continue
            for name, path in files(directory).items():
                if name.endswith((".json", ".toml", ".sha256")):
                    copy_file(path, provenance_files / directory.name / name, False)
        abi_evidence = elf_dependencies(staged)
        host = {
            "system": platform.system(),
            "machine": platform.machine(),
            "libc": list(platform.libc_ver()),
            "cc_version": subprocess.check_output(["cc", "--version"], text=True).splitlines()[0],
            "rustc_versions": {
                toolchain: subprocess.check_output(
                    ["rustc", "+" + toolchain, "--version"], text=True
                ).strip()
                for toolchain in ("1.93.0", "1.97.1")
            },
        }
        return assemble(
            staged,
            output,
            product_version,
            release_id,
            source_commit,
            args.arch,
            epoch,
            args.min_installer_version,
            args.artifact_base_url,
            {
                "input_kind": "source_build",
                "source_commit": source_commit,
                "source_date_epoch": epoch,
                "components": build_inputs,
                "native_build_host": host,
                "elf_dependencies": abi_evidence,
            },
            provenance_files,
        )
    finally:
        cleanup = subprocess.run(
            ["git", "-C", str(source), "worktree", "remove", "--force", str(snapshot)], check=False
        )
        if cleanup.returncode:
            print(f"WARNING: could not remove owned source worktree {snapshot}", file=sys.stderr)


def parse_args() -> argparse.Namespace:
    """Expose source building and explicit staged input as distinct operations."""
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--source-repo", type=Path, default=SOURCE_DEFAULT)
    parser.add_argument(
        "--work-dir", type=Path, help="new isolated build directory; retained for diagnostics"
    )
    parser.add_argument(
        "--staged-input",
        type=Path,
        help="assemble complete prebuilt AW package directories; do not compile",
    )
    parser.add_argument("--product-version", help="defaults to the independent ANOLISA CLI version")
    parser.add_argument("--release-id", help="defaults to PRODUCT_VERSION-SOURCE_COMMIT[:12]")
    parser.add_argument(
        "--source-commit", help="required in staged mode; source mode always pins HEAD"
    )
    parser.add_argument(
        "--arch",
        choices=("x86_64", "aarch64"),
        default={"arm64": "aarch64"}.get(platform.machine(), platform.machine()),
    )
    parser.add_argument(
        "--epoch", type=int, help="required in staged mode; source mode pins commit time"
    )
    parser.add_argument("--min-installer-version")
    parser.add_argument(
        "--artifact-base-url", help="optional HTTPS base URL for release-index.json"
    )
    return parser.parse_args()


def main() -> None:
    """Return failure if any component, ownership or publication check fails."""
    args = parse_args()
    try:
        if args.staged_input:
            require(
                args.source_commit is not None and COMMIT.fullmatch(args.source_commit) is not None,
                "staged mode requires --source-commit",
            )
            require(args.epoch is not None, "staged mode requires --epoch")
            require(args.work_dir is None, "--work-dir is only used by source builds")
            packages = args.staged_input
            cli = read_json(packages / "anolisa-cli/package.json")
            product_version = args.product_version or version(cli["version"])
            release_version = product_version.lower().replace("+", "_")
            release_id = args.release_id or f"{release_version}-{args.source_commit[:12]}"
            result = assemble(
                packages,
                args.output_dir,
                product_version,
                release_id,
                args.source_commit,
                args.arch,
                args.epoch,
                args.min_installer_version,
                args.artifact_base_url,
            )
        else:
            result = source_build(args)
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    print(result)


if __name__ == "__main__":
    main()
