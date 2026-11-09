"""Linux executable lifecycle. Imported by the pinned SSH worker, never root exec.

The caller owns routing and durable operation intent. This adapter starts an
immutable release instance without replacing or stopping an older instance.
Readiness is a local observation, not runtime attestation.
"""
import hashlib
import http.client
import json
import os
import pathlib
import pwd
import re
import socket
import stat
import subprocess
import time

MAX_ENV_ENTRIES = 128
MAX_ENV_NAME = 128
MAX_ENV_VALUE = 8192
MAX_ENV_BYTES = 32768


def require(condition, message):
    if not condition:
        raise ValueError(message)


def identity(path):
    info = os.stat(path)
    return info.st_dev, info.st_ino


def read_owned(path, mode=None):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_uid == 0
                and info.st_nlink == 1 and not info.st_mode & 0o022,
                "runtime input must be an exclusively root-owned regular file")
        require(mode is None or stat.S_IMODE(info.st_mode) == mode, "runtime input mode differs")
        data = stream.read(64 * 1024 * 1024 + 1)
        require(len(data) <= 64 * 1024 * 1024, "runtime input too large")

        return data


def printable_environment_character(character):
    """Match the Rust contract; systemd cannot represent control/noncharacters."""
    code = ord(character)

    return (code >= 32 and not 0x7f <= code <= 0x9f
            and not 0xd800 <= code <= 0xdfff and code != 0xfeff
            and not 0xfdd0 <= code <= 0xfdef and code & 0xffff < 0xfffe)


def environment_valid(values):
    """Bound the decoded values before encoding them into unit-file directives."""
    require(isinstance(values, dict) and len(values) <= MAX_ENV_ENTRIES, "invalid environment entry count")
    total = 0

    for name, value in values.items():
        require(isinstance(name, str) and len(name) <= MAX_ENV_NAME
                and re.fullmatch(r"[A-Z_][A-Z0-9_]*", name), "invalid environment variable name")
        require(isinstance(value, str) and all(map(printable_environment_character, value)),
            "environment values must be printable Unicode")
        size = len(value.encode("utf-8"))
        require(size <= MAX_ENV_VALUE, "environment value exceeds 8192 bytes")
        total += len(name) + size + 2

    require(total <= MAX_ENV_BYTES, "environment exceeds 32768 bytes")


# Keep existing simple units byte-identical for recovery. systemd.syntax
# unquotes C escapes; systemd.exec then expands % specifiers, never $.
def environment_line(name, value):
    if value and all(c.isascii() and (c.isalnum() or c in "_./:@-,") for c in value):
        return f"Environment={name}={value}\n"

    escaped = value.replace("\\", "\\\\").replace('"', '\\"').replace("%", "%%")

    return f'Environment="{name}={escaped}"\n'


def validate(contract):
    require(set(contract) == {"account", "address", "address_env", "environment", "backend",
                              "health", "ready", "runtime"}, "incomplete executable contract")
    require(re.fullmatch(r"[a-z_][a-z0-9_-]{0,30}", contract["account"])
            and contract["account"] != "leptoskit-edge", "application requires a separate account")
    require(isinstance(contract["address_env"], str)
            and re.fullmatch(r"[A-Z_][A-Z0-9_]*", contract["address_env"]),
            "declare a valid application listen environment variable")

    match = re.fullmatch(r"127\.0\.0\.1:([0-9]+)", contract["address"])
    require(match and 1024 <= int(match[1]) <= 65535 and int(match[1]) not in (8088, 18088),
            "application must use a nonreserved private loopback port")
    environment_valid(contract["environment"])
    require(contract["environment"].get(contract["address_env"]) == contract["address"],
            "application listen environment differs from declared socket")

    for endpoint in (contract["health"], contract["ready"]):
        require(re.fullmatch(r"/[A-Za-z0-9_/-]+", endpoint), "unsafe readiness endpoint")
    backend = contract["backend"]
    require(set(backend) == {"sha256", "size", "mode"} and backend["mode"] == 0o755
            and 0 < backend["size"] <= 64 * 1024 * 1024
            and re.fullmatch(r"[a-f0-9]{64}", backend["sha256"]), "invalid executable pin")
    # Runtime installation, private socket/image consumption and compilation
    # child isolation are separate contracts. Never silently omit them.
    require(contract["runtime"] is None, "rootless runtime consumption is not implemented")


class Processes:
    validate = staticmethod(validate)

    def __init__(self, command=None, proc=pathlib.Path("/proc"), accounts=pwd.getpwnam,
                 read=read_owned, pause=time.sleep):
        self.command = command or self.run
        self.proc, self.accounts, self.read, self.pause = pathlib.Path(proc), accounts, read, pause

    @staticmethod
    def run(args):
        return subprocess.run(args, check=True, stdout=subprocess.PIPE,
                              stderr=subprocess.DEVNULL, timeout=30).stdout

    @staticmethod
    def name(release):
        return "leptoskit-app-" + hashlib.sha256(str(release).encode()).hexdigest() + ".service"

    def check(self, release, contract):
        validate(contract)
        account = self.accounts(contract["account"])
        require(account.pw_uid != 0 and account.pw_gid != 0, "application account must not be root")
        data = self.read(release / "backend", 0o755)
        require(len(data) == contract["backend"]["size"]
                and hashlib.sha256(data).hexdigest() == contract["backend"]["sha256"],
                "installed executable bytes differ")
        require(json.loads(self.read(release / "process.json", 0o600)) == contract,
                "installed process contract differs")

        return account

    def unit(self, release, contract):
        validate(contract)
        require(release.is_absolute() and ".." not in release.parts
                and re.fullmatch(r"/[A-Za-z0-9_./-]+", str(release)), "unsafe executable directory")
        environment = "".join(
            environment_line(name, value)
            for name, value in sorted(contract["environment"].items()))
        return ("[Unit]\nAfter=network.target\n[Service]\n"
                f"User={contract['account']}\nGroup={contract['account']}\n"
                f"ExecStart={release}/backend\nWorkingDirectory={release}\n" + environment +
                "Restart=on-failure\nNoNewPrivileges=true\nProtectSystem=strict\n"
                "ProtectHome=true\nPrivateTmp=true\nCapabilityBoundingSet=\n"
                "RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX\n"
                "[Install]\nWantedBy=multi-user.target\n").encode()

    def preflight(self, release, contract, system=pathlib.Path("/etc/systemd/system")):
        """Run before writing application service configuration."""
        self.check(release, contract)
        name = self.name(release)
        path = system / name

        if path.exists():
            require(self.read(path, 0o644) == self.unit(release, contract), "existing application unit differs")
            require(not self.command(["systemctl", "show", "-p", "DropInPaths", "--value", name]).strip(),
                    "application unit has unsupported drop-ins")
        else:
            with socket.socket() as listener:
                try:
                    listener.bind(("127.0.0.1", int(contract["address"].split(":")[1])))

                except OSError as error:
                    raise ValueError("candidate port is occupied; keep the previous release and select a free port") from error

        return path

    def start(self, release, contract, atomic, system=pathlib.Path("/etc/systemd/system")):
        path = self.preflight(release, contract, system)

        if not path.exists():
            atomic(path, self.unit(release, contract), 0o644)
        self.command(["systemctl", "daemon-reload"])
        self.preflight(release, contract, system)
        self.command(["systemctl", "start", self.name(release)])

        return self.observe(release, contract)

    def process_identity(self, pid, release, uid):
        process = self.proc / str(pid)
        require(pid > 0 and process.stat().st_uid == uid
                and identity(process / "exe") == identity(release / "backend"),
                "candidate process does not match executable/account")
        # comm may contain spaces or parentheses. starttime is field 22.
        return process.joinpath("stat").read_bytes().rsplit(b")", 1)[1].split()[19]

    def owns_listener(self, pid, port):
        descriptors = set()

        for entry in (self.proc / str(pid) / "fd").iterdir():
            try:
                target = os.readlink(entry)

            except FileNotFoundError:
                # A process may close an fd while its live descriptor list is read.
                continue

            if target.startswith("socket:["):
                descriptors.add(target[8:-1])

        for line in (self.proc / "net/tcp").read_text().splitlines()[1:]:
            fields = line.split()
            if (fields[1] == f"0100007F:{port:04X}" and fields[3] == "0A"
                    and fields[9] in descriptors):
                return fields[9]
        raise ValueError("candidate does not own the exact loopback listening socket")

    @staticmethod
    def probe(port, endpoint):
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
        try:
            connection.request("GET", endpoint)
            response = connection.getresponse()
            require(response.status == 200 and len(response.read(65537)) <= 65536,
                    "candidate health/readiness response failed")
        finally:
            connection.close()

    def observe(self, release, contract):
        account = self.check(release, contract)
        name = self.name(release)
        port = int(contract["address"].split(":")[1])

        for attempt in range(30):
            try:
                pid = int(self.command(["systemctl", "show", "-p", "MainPID", "--value", name]))
                started = self.process_identity(pid, release, account.pw_uid)
                inode = self.owns_listener(pid, port)
                self.probe(port, contract["health"])
                self.probe(port, contract["ready"])
                require(self.process_identity(pid, release, account.pw_uid) == started
                        and self.owns_listener(pid, port) == inode
                        and int(self.command(["systemctl", "show", "-p", "MainPID", "--value", name])) == pid,
                        "candidate changed during readiness observation")

                self.check(release, contract)

                return {"kind": "local_readiness_observation", "pid": pid,
                        "starttime": started.decode(), "socket_inode": inode,
                        "executable_sha256": contract["backend"]["sha256"], "attestation": "absent"}

            except (OSError, ValueError, http.client.HTTPException) as error:
                if attempt == 29:
                    raise RuntimeError("candidate not ready: verify listen contract, loader, libraries and systemd status") from error
                self.pause(0.1)

    def enable(self, release, contract):
        observation = self.observe(release, contract)
        self.command(["systemctl", "enable", self.name(release)])

        return observation

    def stop(self, release):
        # Only the exact journal-owned release instance is eligible for cleanup.
        self.command(["systemctl", "stop", self.name(release)])
        self.command(["systemctl", "disable", self.name(release)])

    def stop_if_installed(self, release, contract, system=pathlib.Path("/etc/systemd/system")):
        """A failed start may have written no unit; never stop a foreign unit."""
        self.check(release, contract)
        path = system / self.name(release)

        if path.exists():
            require(self.read(path, 0o644) == self.unit(release, contract),
                    "retiring application unit differs")
            require(not self.command(["systemctl", "show", "-p", "DropInPaths", "--value",
                                      self.name(release)]).strip(), "retiring application has drop-ins")
            self.stop(release)
