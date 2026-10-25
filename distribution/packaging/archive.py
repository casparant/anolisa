#!/usr/bin/env python3
"""Archive a staged raw component reproducibly without requiring GNU tar."""

from __future__ import annotations

import argparse
import gzip
import os
import stat
import tarfile
from pathlib import Path


def archive(source: Path, output: Path, epoch: int) -> None:
    """Write sorted ordinary members with fixed metadata, refusing links and special files."""
    if not source.is_dir() or source.is_symlink() or not 0 <= epoch <= 0x7FFFFFFF:
        raise ValueError("invalid archive source or SOURCE_DATE_EPOCH")
    members = [(".", source)]
    for directory, names, filenames in os.walk(source, followlinks=False):
        for name in sorted(names + filenames):
            path = Path(directory) / name
            metadata = path.lstat()
            if not (stat.S_ISDIR(metadata.st_mode) or stat.S_ISREG(metadata.st_mode)):
                raise ValueError(f"unsupported archive member: {path}")
            members.append(("./" + path.relative_to(source).as_posix(), path))
    try:
        with output.open("xb") as stream, gzip.GzipFile(
            filename="", mode="wb", fileobj=stream, mtime=0, compresslevel=9
        ) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.GNU_FORMAT) as target:
                for name, path in sorted(members):
                    metadata = path.lstat()
                    member = tarfile.TarInfo(name)
                    member.uid = member.gid = 0
                    member.mtime = epoch
                    member.uname = member.gname = ""
                    if stat.S_ISDIR(metadata.st_mode):
                        member.type, member.mode = tarfile.DIRTYPE, 0o755
                        target.addfile(member)
                    else:
                        member.mode = 0o755 if metadata.st_mode & 0o111 else 0o644
                        member.size = metadata.st_size
                        with path.open("rb") as incoming:
                            target.addfile(member, incoming)
    except BaseException:
        # Existing outputs are not ours; an exclusive-create failure must preserve them.
        if "stream" in locals():
            output.unlink(missing_ok=True)
        raise


def main() -> None:
    """Expose deterministic component archiving to the existing shell package entries."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--epoch", required=True, type=int)
    args = parser.parse_args()
    archive(args.source, args.output, args.epoch)


if __name__ == "__main__":
    main()
