"""Bounded SSH transport. No user SSH config, agent forwarding or raw errors."""
import base64
import json
import os
import pathlib
import selectors
import shlex
import stat
import subprocess
import sys
import tempfile
import time


def exchange(command, request, timeout=180, output_limit=1024 * 1024):
    """Bound output while it arrives, including while uploading the request."""
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + timeout
    output = bytearray()
    pending = memoryview(request)
    try:
        with selectors.DefaultSelector() as ready:
            os.set_blocking(process.stdin.fileno(), False)
            os.set_blocking(process.stdout.fileno(), False)
            ready.register(process.stdout, selectors.EVENT_READ)
            if pending:
                ready.register(process.stdin, selectors.EVENT_WRITE)
            else:
                process.stdin.close()
            while ready.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("SSH deadline exceeded")
                for key, _ in ready.select(remaining):
                    if key.fileobj is process.stdin:
                        written = os.write(process.stdin.fileno(), pending[:65536])
                        pending = pending[written:]
                        if not pending:
                            ready.unregister(process.stdin)
                            process.stdin.close()
                    else:
                        data = os.read(process.stdout.fileno(), min(65536, output_limit + 1 - len(output)))
                        if not data:
                            ready.unregister(process.stdout)
                        else:
                            output.extend(data)
                            if len(output) > output_limit:
                                raise ValueError("remote output exceeds limit")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("SSH deadline exceeded")
            if process.wait(timeout=remaining):
                raise RuntimeError("SSH failed; journal may describe an interrupted operation")
        return bytes(output)
    finally:
        # This is only the SSH process created above, never another task's process.
        if process.poll() is None:
            process.kill()
        process.wait()
        process.stdin.close()
        process.stdout.close()


def main():
    envelope = json.load(sys.stdin)
    host = envelope["host"]
    trust = host["trust"]
    identity = pathlib.Path(trust["identity"])
    info = identity.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
        raise ValueError("identity must be a private regular file")
    with tempfile.TemporaryDirectory(prefix="lk-ssh-") as private:
        known = pathlib.Path(private) / "known_hosts"
        endpoint = trust["ipv4"]
        known.write_text(f"[{endpoint}]:{trust['port']} {trust['host_key']}\n"
                         f"{endpoint} {trust['host_key']}\n")
        known.chmod(0o600)
        worker = base64.b64encode(envelope["worker"].encode()).decode()
        remote = "python3 -c " + shlex.quote("import base64;exec(base64.b64decode(" + repr(worker) + "))")
        command = ["ssh", "-F", "/dev/null", "-o", "BatchMode=yes",
                   "-o", "StrictHostKeyChecking=yes", "-o", f"UserKnownHostsFile={known}",
                   "-o", "GlobalKnownHostsFile=/dev/null", "-o", "IdentitiesOnly=yes",
                   "-o", "IdentityAgent=none", "-o", "ForwardAgent=no",
                   "-o", "ClearAllForwardings=yes", "-o", "ConnectionAttempts=1",
                   "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=10",
                   "-o", "ServerAliveCountMax=2", "-i", str(identity),
                   "-p", str(trust["port"]), "--", "root@" + endpoint, remote]
        request = json.dumps({"host": host, **envelope["request"]}).encode()
        if len(request) > 96 * 1024 * 1024:
            raise ValueError("remote request exceeds limit")
        value = json.loads(exchange(command, request))
        if not isinstance(value, dict) or value.get("error"):
            raise RuntimeError("remote operation failed")
        expected = {"prepare": ("prepared", "complete"), "activate": ("complete",)}
        phases = expected.get(envelope["request"]["action"])
        if phases and (value.get("phase") not in phases
                       or value.get("operation") != envelope["request"]["operation"]):
            raise RuntimeError("remote operation did not reach the requested state")
        print(json.dumps(value))


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("Graph transport failed; inspect host journal and retry the same checked plan.", file=sys.stderr)
        sys.exit(1)
