#!/usr/bin/env python3
"""Create a reproducible lurkline release archive and checksum."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import re
import tarfile
from pathlib import Path

PLATFORMS = {"linux-x86_64", "linux-aarch64", "macos-aarch64"}
VERSION_PATTERN = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--platform", required=True, choices=sorted(PLATFORMS))
    parser.add_argument("--output-dir", required=True, type=Path)
    return parser.parse_args()


def add_path(
    archive: tarfile.TarFile,
    source: Path,
    archive_path: str,
    mode: int,
) -> None:
    info = archive.gettarinfo(str(source), arcname=archive_path)
    info.uid = 0
    info.gid = 0
    info.uname = "root"
    info.gname = "root"
    info.mtime = 0
    info.mode = mode
    with source.open("rb") as source_file:
        archive.addfile(info, source_file)


def create_archive(
    binary: Path,
    version: str,
    platform: str,
    output_dir: Path,
) -> tuple[Path, Path]:
    if not VERSION_PATTERN.fullmatch(version):
        raise ValueError("version must use MAJOR.MINOR.PATCH")
    if platform not in PLATFORMS:
        raise ValueError(f"unsupported platform: {platform}")
    if not binary.is_file():
        raise FileNotFoundError(f"binary does not exist: {binary}")

    repository_root = Path(__file__).resolve().parent.parent
    readme = repository_root / "README.md"
    license_file = repository_root / "LICENSE"
    for required_file in (readme, license_file):
        if not required_file.is_file():
            raise FileNotFoundError(f"required package file does not exist: {required_file}")

    basename = f"lurkline-v{version}-{platform}"
    output_dir.mkdir(parents=True, exist_ok=True)
    archive_path = output_dir / f"{basename}.tar.gz"
    checksum_path = output_dir / f"{archive_path.name}.sha256"

    tar_buffer = io.BytesIO()
    with tarfile.open(fileobj=tar_buffer, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        directory = tarfile.TarInfo(basename)
        directory.type = tarfile.DIRTYPE
        directory.mode = 0o755
        directory.uid = 0
        directory.gid = 0
        directory.uname = "root"
        directory.gname = "root"
        directory.mtime = 0
        archive.addfile(directory)

        add_path(archive, binary, f"{basename}/lurkline", 0o755)
        add_path(archive, readme, f"{basename}/README.md", 0o644)
        add_path(archive, license_file, f"{basename}/LICENSE", 0o644)

    with archive_path.open("wb") as output_file:
        with gzip.GzipFile(filename="", mode="wb", fileobj=output_file, mtime=0) as compressed:
            compressed.write(tar_buffer.getvalue())

    digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    checksum_path.write_text(f"{digest}  {archive_path.name}\n", encoding="ascii")
    return archive_path, checksum_path


def main() -> None:
    args = parse_args()
    archive_path, checksum_path = create_archive(
        args.binary.resolve(),
        args.version,
        args.platform,
        args.output_dir.resolve(),
    )
    print(archive_path)
    print(checksum_path)


if __name__ == "__main__":
    main()
