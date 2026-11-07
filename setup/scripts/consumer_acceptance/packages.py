"""Extract exact Cargo package payloads without using repository source paths."""

import hashlib
import json
from pathlib import PurePosixPath
import shutil
import tarfile
import tomllib


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def extract_archive(archive, output, name, version, revision):
    root_name = f"{name}-{version}"
    archive_hash = sha256(archive)
    with tarfile.open(archive, "r:gz") as bundle:
        for member in bundle:
            path = PurePosixPath(member.name)
            if (
                path.is_absolute()
                or ".." in path.parts
                or not path.parts
                or path.parts[0] != root_name
                or not (member.isfile() or member.isdir())
            ):
                raise ValueError(f"{archive.name}: unsupported archive member {member.name}")
            target = output.joinpath(*path.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with bundle.extractfile(member) as source, target.open("xb") as destination:
                    shutil.copyfileobj(source, destination)
                target.chmod(0o755 if member.mode & 0o111 else 0o644)
    directory = (output / root_name).resolve()
    manifest = tomllib.loads((directory / "Cargo.toml").read_text())
    package = manifest["package"]
    if package["name"] != name or package["version"] != version:
        raise ValueError(f"{archive.name}: package identity mismatch")
    vcs = json.loads((directory / ".cargo_vcs_info.json").read_text())
    if vcs["git"]["sha1"] != revision:
        raise ValueError(f"{archive.name}: package records a different source revision")
    if sha256(archive) != archive_hash:
        raise ValueError(f"{archive.name}: package archive changed during extraction")
    return {"version": version, "path": directory, "sha256": archive_hash}


def load_packages(metadata, archives, output, revision):
    expected = [package for package in metadata["packages"] if package.get("publish") != []]
    if not expected or not any(package["name"] == "leptoskit" for package in expected):
        raise ValueError("no publishable facade in workspace metadata")
    packages = {}
    for package in expected:
        name, version = package["name"], package["version"]
        if not name.startswith("leptoskit"):
            raise ValueError(f"unexpected publishable package {name}")
        archive = archives / f"{name}-{version}.crate"
        if not archive.is_file():
            raise ValueError(f"missing package archive {archive.name}; run cargo package first")
        packages[name] = extract_archive(archive, output, name, version, revision)
    return packages
