#!/usr/bin/env python3
"""Publish a deterministic complete ANOLISA tarball from explicit AW package inputs."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import os
import shutil
import stat
import sys
import tarfile
import tempfile
from pathlib import Path
from typing import Any
from urllib.parse import quote, urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[4]))
from distribution.packaging.product import distribution
from distribution.packaging.recipes import package_specs

from release_common import (
    COMMIT,
    digest,
    files,
    identifier,
    json_bytes,
    no_links,
    regular,
    require,
    validate_packages,
    version,
)

BASE = Path(__file__).resolve().parent


def copy_regular(source: Path, destination: Path, mode: int) -> None:
    """Copy only a checked ordinary file into a private unpublished directory."""
    regular(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with source.open("rb") as incoming, destination.open("xb") as outgoing:
        shutil.copyfileobj(incoming, outgoing, 1024**2)
    destination.chmod(mode)


def entrypoints(specs: list[dict[str, Any]], packages: list[dict[str, Any]]) -> dict[str, str]:
    """Resolve public commands only after their unique package ownership is verified."""
    by_id = {package["package_id"]: package for package in packages}
    result: dict[str, str] = {}
    for spec in specs:
        for command, target in spec["entrypoints"].items():
            identifier(command)
            require(command not in result, f"duplicate public command: {command}")
            metadata = by_id[spec["package_id"]]["files"].get(target)
            require(
                target.startswith("bin/") and metadata is not None and metadata["mode"] == 0o755,
                f"public command is not an owned executable: {command}",
            )
            result[command] = target
    require({"aw", "anolisa", "cosh"} <= result.keys(), "missing core product commands")
    return dict(sorted(result.items()))


def archive_tree(root: Path, destination: Path, top: str, epoch: int) -> None:
    """Sort members and normalize ownership, times and gzip metadata."""
    members = {"": root}
    for directory, names, filenames in os.walk(root):
        for name in names + filenames:
            path = Path(directory) / name
            members[path.relative_to(root).as_posix()] = path
    with destination.open("xb") as raw, gzip.GzipFile(
        filename="", fileobj=raw, mode="wb", compresslevel=9, mtime=0
    ) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.GNU_FORMAT) as archive:
            for relative, path in sorted(members.items()):
                metadata = path.lstat()
                name = top + ("/" + relative if relative else "")
                member = tarfile.TarInfo(name)
                member.mtime = epoch
                member.uid = member.gid = 0
                member.uname = member.gname = ""
                if stat.S_ISDIR(metadata.st_mode):
                    member.type, member.mode = tarfile.DIRTYPE, 0o755
                    archive.addfile(member)
                else:
                    regular(path)
                    member.mode, member.size = stat.S_IMODE(metadata.st_mode), metadata.st_size
                    with path.open("rb") as stream:
                        archive.addfile(member, stream)


def artifact_url(base: str, filename: str) -> str:
    """Require an explicit credential-free publication URL; never invent a mirror."""
    parsed = urlsplit(base)
    require(
        parsed.scheme == "https"
        and bool(parsed.netloc)
        and parsed.username is None
        and parsed.password is None
        and not parsed.query
        and not parsed.fragment,
        "artifact-base-url must be an absolute credential-free HTTPS URL",
    )
    require(not any(ord(char) < 33 for char in base), "whitespace in artifact-base-url")
    return base.rstrip("/") + "/" + quote(filename, safe="")


def assemble(
    packages_dir: Path,
    output_dir: Path,
    product_version: str,
    release_id: str,
    source_commit: str,
    arch: str,
    epoch: int,
    min_installer_version: str | None = None,
    artifact_base_url: str | None = None,
    provenance: dict[str, Any] | None = None,
    provenance_files: Path | None = None,
    recipe_source: Path | None = None,
) -> Path:
    """Atomically publish the archive, checksum and optional publication index together."""
    version(product_version)
    identifier(release_id)
    require(COMMIT.fullmatch(source_commit) is not None, "invalid source_commit")
    require(type(epoch) is int and 0 <= epoch <= 0x7FFFFFFF, "invalid SOURCE_DATE_EPOCH")
    recipe_source = recipe_source or BASE.parents[3]
    specs = package_specs(recipe_source)
    packages_dir = packages_dir.absolute()
    output_dir = output_dir.absolute()
    no_links(packages_dir)
    no_links(output_dir)
    require(not output_dir.is_relative_to(packages_dir), "output must be outside staged inputs")
    ordered = validate_packages(packages_dir, specs, arch, source_commit)
    installer = next(package for package in ordered if package["package_id"] == "anolisa-cli")
    minimum = version(min_installer_version or installer["version"])
    public = entrypoints(specs, ordered)
    filename = f"anolisa-{product_version}-linux-{arch}-gnu.tar.gz"
    url = artifact_url(artifact_base_url, filename) if artifact_base_url else None
    require(not output_dir.exists(), f"refusing to replace an existing output: {output_dir}")
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    no_links(output_dir.parent)
    lock = output_dir.parent / ("." + output_dir.name + ".lock")
    descriptor = os.open(lock, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        require(not output_dir.exists(), f"output appeared while locking: {output_dir}")
        with tempfile.TemporaryDirectory(prefix=".anolisa-release-", dir=output_dir.parent) as work:
            work_root = Path(work)
            publish = work_root / "publish"
            publish.mkdir(mode=0o755)
            bundle = work_root / "bundle"
            bundle.mkdir(mode=0o755)
            for package in ordered:
                package_id = package["package_id"]
                source = packages_dir / package_id
                target = bundle / "packages" / package_id
                copy_regular(source / "package.json", target / "package.json", 0o644)
                for name, metadata in package["files"].items():
                    copy_regular(
                        source / "payload" / name, target / "payload" / name, metadata["mode"]
                    )
            # Validate the copied bytes, not just the caller's mutable input directory.
            validate_packages(bundle / "packages", specs, arch, source_commit)
            release = {
                "schema_version": 2,
                "product_version": product_version,
                "release_id": release_id,
                "source_commit": source_commit,
                "os": "linux",
                "arch": arch,
                "abi": "gnu",
                "min_installer_version": minimum,
                "packages": [
                    {
                        "package_id": package["package_id"],
                        "version": package["version"],
                        "path": "packages/" + package["package_id"],
                        "description_sha256": digest(
                            bundle / "packages" / package["package_id"] / "package.json"
                        ),
                    }
                    for package in ordered
                ],
                "entrypoints": public,
                "distribution": distribution(recipe_source),
            }
            (bundle / "release.json").write_bytes(json_bytes(release))
            (bundle / "release.json").chmod(0o644)
            copy_regular(recipe_source / "distribution/anolisa/packaging/release/install.sh", bundle / "install.sh", 0o755)
            (bundle / "provenance").mkdir(mode=0o755)
            if provenance_files is not None:
                for name, path in files(provenance_files).items():
                    copy_regular(path, bundle / "provenance/components" / name, 0o644)
            evidence = provenance or {
                "input_kind": "staged_packages",
                "source_commit": source_commit,
            }
            (bundle / "provenance" / "build-input.json").write_bytes(json_bytes(evidence))
            (bundle / "provenance" / "build-input.json").chmod(0o644)
            checksums = "".join(f"{digest(path)}  {name}\n" for name, path in files(bundle).items())
            (bundle / "SHA256SUMS").write_text(checksums, encoding="utf-8")
            (bundle / "SHA256SUMS").chmod(0o644)
            archive = publish / filename
            archive_tree(bundle, archive, f"anolisa-{release_id}-linux-{arch}", epoch)
            archive.chmod(0o644)
            archive_digest = hashlib.sha256()
            with archive.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024**2), b""):
                    archive_digest.update(chunk)
            checksum = archive_digest.hexdigest()
            (publish / (filename + ".sha256")).write_text(
                f"{checksum}  {filename}\n", encoding="utf-8"
            )
            if url:
                index = {
                    "schema_version": 1,
                    "product_version": product_version,
                    "artifacts": [
                        {
                            "os": "linux",
                            "arch": arch,
                            "abi": "gnu",
                            "url": url,
                            "sha256": checksum,
                            "size": archive.stat().st_size,
                        }
                    ],
                }
                (publish / "release-index.json").write_bytes(json_bytes(index))
            for path in publish.iterdir():
                path.chmod(0o644)
                with path.open("rb") as stream:
                    os.fsync(stream.fileno())
            require(not output_dir.exists(), f"output appeared before publication: {output_dir}")
            os.rename(publish, output_dir)
        return output_dir / filename
    finally:
        os.close(descriptor)
        lock.unlink()


def parse_args() -> argparse.Namespace:
    """Parse the explicit prebuilt-input contract without a hidden source build."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--staged-input",
        required=True,
        type=Path,
        help="directory containing the complete set of AW package directories",
    )
    parser.add_argument(
        "--output-dir",
        required=True,
        type=Path,
        help="new directory published atomically; existing outputs are never replaced",
    )
    parser.add_argument("--product-version", required=True)
    parser.add_argument("--release-id", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--arch", required=True, choices=("x86_64", "aarch64"))
    parser.add_argument("--epoch", required=True, type=int)
    parser.add_argument("--min-installer-version")
    parser.add_argument("--artifact-base-url")
    return parser.parse_args()


def main() -> None:
    """Report a real input-validation or publication failure with a nonzero status."""
    args = parse_args()
    try:
        result = assemble(
            args.staged_input,
            args.output_dir,
            args.product_version,
            args.release_id,
            args.source_commit,
            args.arch,
            args.epoch,
            args.min_installer_version,
            args.artifact_base_url,
        )
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    print(result)


if __name__ == "__main__":
    main()
