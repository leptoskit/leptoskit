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
    seen_paths = {}
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
            for length in range(1, len(path.parts) + 1):
                prefix = str(PurePosixPath(*path.parts[:length]))
                previous = seen_paths.setdefault(prefix.casefold(), prefix)
                if previous != prefix:
                    raise ValueError(
                        f"{archive.name}: case-insensitive path collision: {previous} and {prefix}"
                    )
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


def verify_report(report, metadata, archives, revision, presets):
    """Match a successful full consumer run to freshly prepared upload archives."""
    if (report.get("format_version") != 1 or report.get("state") != "passed"
            or report.get("source_revision") != revision):
        raise ValueError("consumer report does not match this source revision")
    consumers = report.get("consumers", [])
    if len(consumers) != len(presets) or {item.get("preset") for item in consumers} != set(presets):
        raise ValueError("consumer report is missing required profiles or repeats one")
    for consumer in consumers:
        features, target = presets[consumer["preset"]]
        if (consumer.get("state") != "passed" or consumer.get("features") != features
                or consumer.get("target") != (target or "host")):
            raise ValueError(f"consumer report has incomplete evidence for {consumer['preset']}")
    expected = {item["name"]: item["version"] for item in metadata["packages"]
                if item.get("publish") != []}
    tested = report.get("packages", {})
    if not expected or "leptoskit" not in expected or set(tested) != set(expected):
        raise ValueError("consumer report package set differs from the publication")
    for name, version in expected.items():
        archive = archives / f"{name}-{version}.crate"
        if tested[name].get("version") != version or sha256(archive) != tested[name].get("sha256"):
            raise ValueError(f"{name}: prepared archive differs from the tested payload")
