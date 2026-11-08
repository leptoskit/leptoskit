"""Fake SSH/engine for command routing tests, not Linux installer acceptance."""

import hashlib
import json
import os
import pathlib
import re
import shlex
import stat
import sys
import tempfile

root = pathlib.Path(os.environ["MOCK_REMOTE_ROOT"])
script = sys.argv[1]


def local(path):
    return root / "state" / path.removeprefix("/var/lib/leptoskit/")


def checked_file(path):
    assert not path.is_symlink() and path.is_file()
    assert stat.S_IMODE(path.stat().st_mode) == 0o444
    return hashlib.sha256(path.read_bytes()).hexdigest()


if "\ntemporary=$(mktemp" in script:
    assignments = dict(
        (key, shlex.split(value)[0])
        for key, value in re.findall(r"^(directory|file|pin)=(.+)$", script, re.M)
    )
    directory = local(assignments["directory"])
    path = local(assignments["file"])
    content = sys.stdin.buffer.read()
    assert hashlib.sha256(content).hexdigest() == assignments["pin"]
    if os.environ.get("MOCK_WINDOW_INSTALL_FAIL"):
        sys.exit(1)
    directory.mkdir(parents=True, exist_ok=True)
    directory.chmod(0o700)
    with tempfile.NamedTemporaryFile(dir=directory) as temporary:
        temporary.write(content)
        temporary.flush()
        os.chmod(temporary.name, 0o444)
        try:
            os.link(temporary.name, path, follow_symlinks=False)
        except FileExistsError:
            pass
        assert checked_file(path) == assignments["pin"]
    if os.environ.get("MOCK_WINDOW_LOSE_RESERVATION"):
        (directory.parent / "operation.lock").write_text("another-owner")
    print(assignments["pin"], end="")
elif script.startswith("docker inspect "):
    container = shlex.split(script)[-1]
    version = container.removeprefix("notes-production-web-")
    runtime = root.parent / "infra/notes-production" / f"boot-window-{version}.json"
    config = json.loads(runtime.read_text())
    environment = [f"{name}={value}" for name, value in config["env"]["clear"].items()]
    mounts = []
    for volume in config.get("volumes", []):
        source, destination, mode = volume.split(":")
        mounts.append({"Type": "bind", "RW": mode != "ro", "Source": source, "Destination": destination})
    bad = os.environ.get("MOCK_WINDOW_OBSERVATION")
    if bad == "pin":
        environment = [entry for entry in environment if not entry.startswith("LEPTOSKIT_MIGRATION_WINDOW_SHA256=")]
        environment.append("LEPTOSKIT_MIGRATION_WINDOW_SHA256=" + "0" * 64)
    if bad == "missing-env":
        environment = [entry for entry in environment if not entry.startswith("LEPTOSKIT_MIGRATION_WINDOW=")]
    if bad == "missing-mount":
        mounts = []
    if bad == "source":
        mounts[0]["Source"] += ".substituted"
    if bad == "mount":
        mounts[0]["RW"] = True
    print(json.dumps([{"State": {"Running": True}, "Config": {"Env": environment}, "Mounts": mounts}]))
else:
    path = local(re.search(r"'(/var/lib/leptoskit/[^']+/schema-windows/[^']+)'", script).group(1))
    digest = checked_file(path)
    print("0" * 64 if os.environ.get("MOCK_WINDOW_OBSERVATION") == "file" else digest)
