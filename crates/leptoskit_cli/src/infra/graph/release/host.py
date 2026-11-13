"""Linux static-release state machine, also exercised with a deterministic runtime.
Only the checked operator CLI sends requests. Root is the trusted installer;
public Caddy processes use an existing dedicated unprivileged account.
"""
import base64
import copy
import fcntl
import hashlib
import http.client
import json
import os
import pathlib
import pwd
import re
import stat
import ssl
import socket
import subprocess
import sys
import tempfile
import time


try:
    Processes
except NameError:
    from process import Processes, masked_dropins

try:
    Instances
except NameError:
    from instances import Instances, render_edge, edge_addresses


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sync(directory):
    fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic(path, data, mode=0o600):
    fd, temporary = tempfile.mkstemp(prefix=".write-", dir=path.parent)
    try:
        os.fchmod(fd, mode)
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)

        sync(path.parent)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def safe_directory(path, create=False, mode=0o755):
    """All parents belong to the installer, with no writable aliases."""
    path = pathlib.Path(path)
    require(path.is_absolute() and ".." not in path.parts, "unsafe installation path")

    for part in [*reversed(path.parents), path]:
        if not part.exists() and create:
            part.mkdir(mode=mode if part == path else 0o755)
            part.chmod(mode if part == path else 0o755)
            sync(part.parent)
        info = part.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid in (0, os.geteuid())
                and not info.st_mode & 0o022, "installation parent is not exclusively owned")


def regular(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_uid == os.geteuid()
                and info.st_nlink == 1 and not info.st_mode & 0o022, "unsafe runtime file")

        return stream.read(64 * 1024 * 1024 + 1), stat.S_IMODE(info.st_mode)


def inventory(directory, expected):
    safe_directory(directory)
    found = {}

    for base, dirs, files in os.walk(directory, followlinks=False):
        for name in dirs + files:
            path = pathlib.Path(base) / name
            relative = str(path.relative_to(directory))
            info = path.lstat()
            require(not stat.S_ISLNK(info.st_mode), "linked release member")

            if stat.S_ISDIR(info.st_mode):
                found[relative] = {"kind": "directory", "mode": stat.S_IMODE(info.st_mode)}
            else:
                data, mode = regular(path)
                found[relative] = {"kind": "file", "mode": mode, "size": len(data), "sha256": digest(data)}
    require(found == expected, "copied membership, mode or bytes differ")


def pointer(path):
    if not path.is_symlink():
        require(not path.exists(), "current pointer is not a link")

        return None
    target = os.readlink(path)
    require(re.fullmatch(r"releases/[A-Za-z0-9_-]+", target), "unsafe current pointer")

    return target


def switch(path, target):
    if target is None:
        if path.is_symlink():
            path.unlink()
    else:
        temporary = path.with_name(".next-" + os.urandom(12).hex())
        os.symlink(target, temporary)
        os.replace(temporary, path)
    sync(path.parent)


class Linux:
    def __init__(self):
        self.processes = Processes()

    def command(self, args, **kwargs):
        return subprocess.run(args, check=True, stdout=subprocess.PIPE,
                              stderr=subprocess.DEVNULL, timeout=30, **kwargs).stdout

    def preflight(self, host):
        require(sys.platform == "linux" and os.geteuid() == 0, "Linux root installer required")
        require(os.uname().machine == host["trust"]["architecture"], "host architecture differs")

        if host["edge"] is None:
            self.private_preflight(host)

            return
        user = pwd.getpwnam("leptoskit-edge")
        require(user.pw_uid != 0, "create dedicated leptoskit-edge account first")
        self.user = user

        for path, pin in host["trust"]["prerequisites"].items():
            require(path.startswith("/etc/leptoskit/") and not path.endswith(".key"),
                    "prerequisites must reference configured non-key inputs")
            safe_directory(pathlib.Path(path).parent)
            require(digest(regular(path)[0]) == pin, "TLS/tunnel prerequisite changed or missing")
        tunnel = host["trust"]["tunnel"]

        if tunnel:
            require(re.fullmatch(r"[a-zA-Z0-9_-]+\.service", tunnel["unit"]), "unsafe tunnel unit")
            self.command(["systemctl", "is-active", "--quiet", tunnel["unit"]])
            fragment = self.command(["systemctl", "show", "-p", "FragmentPath", "--value", tunnel["unit"]]).decode().strip()
            drops = self.command(["systemctl", "show", "-p", "DropInPaths", "--value", tunnel["unit"]]).strip()
            require(masked_dropins(drops) and fragment == tunnel["fragment"]
                    and digest(regular(fragment)[0]) == tunnel["sha256"], "tunnel runtime differs")
            require(host["trust"]["prerequisites"], "pin the configured tunnel input")
        service = pathlib.Path("/etc/systemd/system") / self.service(host)

        if service.exists():
            require(regular(service)[0] == self.unit(host), "existing edge service differs")
            drops = self.command(["systemctl", "show", "-p", "DropInPaths", "--value", self.service(host)]).strip()
            require(masked_dropins(drops), "edge service drop-ins are unsupported")
        # Full TLS validation occurs under the Caddy UID before promotion.

    def private_preflight(self, host, system=pathlib.Path("/etc/systemd/system")):
        """A private release cannot adopt or disable an existing public edge."""
        require(not host["trust"]["prerequisites"] and host["trust"]["tunnel"] is None,
                "private host cannot consume edge prerequisites")
        service = self.service(host)
        require(not os.path.lexists(system / service), "private host has an existing edge unit")
        raw = self.command(["systemctl", "show", "--property=LoadState,ActiveState,MainPID", service])
        properties = dict(line.split("=", 1) for line in raw.decode().splitlines() if "=" in line)
        require(properties == {"LoadState": "not-found", "ActiveState": "inactive", "MainPID": "0"},
                "private host has a loaded or active edge unit")

    def demote(self):
        os.setgroups([])
        os.setgid(self.user.pw_gid)
        os.setuid(self.user.pw_uid)

    def service(self, host):
        name = pathlib.Path(host["root"]).name
        require(re.fullmatch(r"[a-zA-Z0-9_-]+", name), "unsafe application name")

        return "leptoskit-edge-" + name + ".service"

    def unit(self, host):
        root = host["root"]
        return (f"[Unit]\nDescription=leptoskit static edge\nAfter=network.target\n"
                f"[Service]\nUser=leptoskit-edge\nGroup=leptoskit-edge\n"
                f"ExecStart={root}/edge/current/caddy run --config {root}/edge/current/config.json\n"
                "Restart=on-failure\nNoNewPrivileges=true\nProtectSystem=strict\n"
                "ProtectHome=true\nPrivateTmp=true\nAmbientCapabilities=CAP_NET_BIND_SERVICE\n"
                "CapabilityBoundingSet=CAP_NET_BIND_SERVICE\n"
                "[Install]\nWantedBy=multi-user.target\n").encode()

    def install_service(self, host):
        path = pathlib.Path("/etc/systemd/system") / self.service(host)

        if path.exists():
            require(regular(path)[0] == self.unit(host), "existing edge service differs")
        else:
            atomic(path, self.unit(host), 0o644)
        self.command(["systemctl", "daemon-reload"])
        drops = self.command(["systemctl", "show", "-p", "DropInPaths", "--value", self.service(host)]).strip()
        require(masked_dropins(drops), "edge service drop-ins are unsupported")

    def owns_listener(self, pid, port):
        sockets = set()

        for entry in (pathlib.Path("/proc") / str(pid) / "fd").iterdir():
            try:
                link = os.readlink(entry)

            except FileNotFoundError:
                continue

            if link.startswith("socket:["):
                sockets.add(link[8:-1])

        for table in ("tcp", "tcp6"):
            for row in pathlib.Path("/proc/net", table).read_text().splitlines()[1:]:
                fields = row.split()
                if (fields[3] == "0A" and int(fields[1].split(":")[-1], 16) == port
                        and fields[9] in sockets):
                    return True
        return False

    def candidate(self, host, edge, releases):
        if host["edge"] is None:
            return

        binary = edge / "caddy"
        version = self.command([str(binary), "version"], preexec_fn=self.demote).decode()
        require(version.split()[0] == "v" + host["caddy_version"], "Caddy version differs")
        self.command([str(binary), "validate", "--config", str(edge / "config.json")], preexec_fn=self.demote)
        value = json.loads(regular(edge / "config.json")[0])
        # Candidate checks use the same generated route tree, replacing only
        # selected static roots and listener/TLS for a private HTTP probe.
        def replace(node):
            if isinstance(node, dict):
                for key, item in node.items():
                    if key == "root" and isinstance(item, str):
                        for unit, release in releases.items():
                            if item == host["root"] + "/units/" + unit + "/current/assets":
                                node[key] = str(release / "assets")
                    else:
                        replace(item)

            elif isinstance(node, list):
                for item in node:
                    replace(item)
        replace(value)
        value["apps"].pop("tls", None)
        server = value["apps"]["http"]["servers"]["edge"]
        server["listen"] = ["127.0.0.1:18088"]
        server.pop("tls_connection_policies", None)

        server.pop("strict_sni_host", None)
        candidate = edge / "candidate.json"
        atomic(candidate, canonical(value), 0o644)
        process = subprocess.Popen([str(binary), "run", "--config", str(candidate)],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                   preexec_fn=self.demote)
        try:
            for attempt in range(30):
                require(process.poll() is None, "candidate exited")
                try:
                    if not self.owns_listener(process.pid, 18088):
                        time.sleep(0.1)
                        continue
                    self.probe(host, releases, 18088)
                    require(process.poll() is None, "candidate exited during probe")

                    return

                except (OSError, http.client.HTTPException):
                    time.sleep(0.1)
            raise RuntimeError("candidate not ready")
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)

            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()

    def probe(self, host, releases, port, tls=False):
        # A selected index must be served by its exact configured domain/mount.
        for unit, release in releases.items():
            if unit not in host.get("routes", {}):
                continue
            route = host["routes"][unit]
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)

            if tls:
                context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
                context.check_hostname = False
                context.verify_mode = ssl.CERT_NONE
                connection = http.client.HTTPSConnection("127.0.0.1", port, timeout=2, context=context)
                connection.sock = context.wrap_socket(socket.create_connection(("127.0.0.1", port), timeout=2), server_hostname=route["domain"])
                # Trust the independently pinned configured leaf certificate,
                # without system CA/network dependencies for origin-only TLS.
                cert = next(item["certificate"] for item in host["edge"]["apps"]["tls"]["certificates"]["load_files"]
                            if item["certificate"].endswith("/" + route["domain"] + ".crt"))
                expected = ssl.PEM_cert_to_DER_cert(regular(cert)[0].decode())

                require(connection.sock.getpeercert(binary_form=True) == expected, "active TLS certificate differs")
            try:
                contract = host.get("executables", {}).get(unit)
                endpoints = [contract["health"], contract["ready"]] if contract else [route["mount"]]

                for endpoint in endpoints:
                    connection.request("GET", endpoint, headers={"Host": route["domain"]})
                    response = connection.getresponse()
                    limit = 65536 if contract else 64 * 1024 * 1024
                    body = response.read(limit + 1)
                    require(response.status == 200 and len(body) <= limit,
                            "candidate route probe failed")

                    if not contract:
                        require(body == regular(release / "assets/index.html")[0],
                                "candidate route did not serve selected index bytes")
            finally:
                connection.close()

    def restart(self, host):
        if host["edge"] is None:
            return

        self.install_service(host)
        self.command(["systemctl", "enable", self.service(host)])
        self.command(["systemctl", "restart", self.service(host)])
        self.command(["systemctl", "is-active", "--quiet", self.service(host)])

    def active_identity(self, host):
        pid = int(self.command(["systemctl", "show", "-p", "MainPID", "--value", self.service(host)]))
        require(pid > 0, "edge service has no main process")
        process = pathlib.Path("/proc") / str(pid)
        executable = os.stat(process / "exe")
        installed = os.stat(pathlib.Path(host["root"]) / "edge/current/caddy")
        require((executable.st_dev, executable.st_ino) == (installed.st_dev, installed.st_ino)
                and os.stat(process).st_uid == self.user.pw_uid,
                "active process differs from installed binary or dedicated UID")
        started = process.joinpath("stat").read_bytes().rsplit(b")", 1)[1].split()[19]

        return pid, started

    def active(self, host, releases):
        if host["edge"] is None:
            return

        tls = "tls" in host["edge"]["apps"]
        port = 443 if tls else 8088
        # Type=simple can report an active systemd executor before exec and
        # credential setup finish. Wait for our binary and account as well
        # as readiness; repeat their identity checks after probing.
        for attempt in range(30):
            try:
                identity = self.active_identity(host)
                pid = identity[0]
                require(self.owns_listener(pid, port), "edge does not own its listening socket")
                self.probe(host, releases, port, tls=tls)
                require(self.active_identity(host) == identity and self.owns_listener(pid, port),
                        "edge process changed during readiness observation")

                return

            except (OSError, ValueError, http.client.HTTPException) as error:
                if attempt == 29:
                    raise RuntimeError("active edge did not become ready") from error
                time.sleep(0.1)

    def stop(self, host, system=pathlib.Path("/etc/systemd/system")):
        if host["edge"] is None:
            return

        # First activation can fail before the edge unit is installed. There
        # is then nothing owned by this operation to stop or disable.
        path = system / self.service(host)

        if not path.exists():
            return
        require(regular(path)[0] == self.unit(host), "retiring edge service differs")
        drops = self.command(["systemctl", "show", "-p", "DropInPaths", "--value", self.service(host)]).strip()
        require(masked_dropins(drops), "retiring edge service has drop-ins")
        self.command(["systemctl", "stop", self.service(host)])
        self.command(["systemctl", "disable", self.service(host)])


class ReleaseOperation:
    """One checked request and its durable per-host transaction context."""
    def __init__(self, request, root, journal, state):
        self.request, self.root, self.journal, self.state = request, root, journal, state
        self.host, self.identifier, self.version = request["host"], request["operation"], request["version"]
        self.binding = digest(canonical(self.host))
        self.releases = {}

        for unit in self.host["units"]:
            directory = root / "units" / unit / "releases"
            safe_directory(directory, create=True)
            self.releases[unit] = directory / self.version
        safe_directory(root / "edge/releases", create=True)
        transaction = state.get("transaction", self.identifier) if state and state["operation"] == self.identifier else self.identifier
        require(re.fullmatch(r"[a-f0-9]{64}", transaction), "invalid transaction identity")
        self.edge = root / "edge/releases" / transaction

        if state and state["operation"] == self.identifier:
            require(state["binding"] == self.binding, "operation binding differs")


class Machine:
    def __init__(self, runtime, fault=lambda point: None):
        self.runtime, self.fault = runtime, fault
        self.instances = Instances(getattr(runtime, "processes", None), regular, atomic)

    def validate_request(self, request):
        """Reject malformed declarations before locks or release state are touched."""
        host, action = request["host"], request["action"]
        require(action in ("prepare", "activate", "status"), "unsupported release action")
        require(type(request.get("rollback", False)) is bool, "invalid rollback intent")
        require(type(request.get("recovery", False)) is bool, "invalid recovery intent")
        self.instances.validate(host)
        self.validate_private(request)
        require(re.fullmatch(r"[a-f0-9]{64}", request["operation"]), "invalid operation")
        require(re.fullmatch(r"[A-Za-z0-9_-]{1,80}", request["version"]), "invalid version")

        for unit, tree in host["units"].items():
            require(re.fullmatch(r"(?:frontends|services)\.[A-Za-z0-9_-]+", unit), "unsupported unit")
            require(not unit.startswith("services.") or unit in host.get("executables", {}),
                    "service requires an executable contract")

            for path in tree:
                require(path and not path.startswith("/") and all(p not in ("", ".", "..") for p in path.split("/")),
                        "unsafe artifact path")

    @staticmethod
    def validate_private(request):
        """No-edge declarations must describe only actual executable units."""
        host = request["host"]

        if host["edge"] is not None:
            return
        require(host["units"] and set(host.get("executables", {})) == set(host["units"]),
                "private host requires an executable for every selected unit")
        require(host.get("routes") == {} and host.get("edge_sha256") is None
                and host.get("caddy_sha256") is None and host.get("caddy_version") is None
                and request.get("caddy") is None, "private host cannot consume an edge runtime")
        require(not host["trust"]["prerequisites"] and host["trust"]["tunnel"] is None,
                "private host cannot consume edge prerequisites")

    def run(self, request):
        self.validate_request(request)
        host, action = request["host"], request["action"]
        root = pathlib.Path(host["root"])

        if action == "status" and not root.exists():
            return {"state": "absent", "runtime": "not_observed"}

        if action != "status":
            require(not request.get("recovery") or root.exists(), "no historical operation to recover")
            self.runtime.preflight(host)
        safe_directory(root, create=action != "status")
        fd = os.open(root / "operation.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "w") as lock:
            require(stat.S_ISREG(os.fstat(fd).st_mode) and os.fstat(fd).st_nlink == 1, "unsafe operation lock")
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

            return self.public_result(self.locked(request, root))

    @staticmethod
    def public_result(result):
        """Keep recovery inventories on-host instead of filling bounded SSH replies."""
        if "journal" in result:
            return {**result, "journal": Machine.public_result(result["journal"])
                    if result["journal"] is not None else None}
        fields = ("operation", "transaction", "version", "phase", "rollback",
                  "observations", "legacy_recovery", "legacy_phase")
        return {name: result[name] for name in fields if name in result}

    def locked(self, request, root):
        """A context must own the journal before it can enter failure cleanup."""
        host, action = request["host"], request["action"]
        journal = root / "journal.json"

        state = json.loads(regular(journal)[0]) if journal.exists() else None

        if action == "status":
            if request.get("recovery"):
                self.validate_context(state, request)
            return {"journal": state, "current": {u: pointer(root / "units" / u / "current")

                    for u in host["units"]}, "runtime": "not_observed"}

        self.validate_context(state, request)
        legacy = self.legacy_context(state)
        if legacy:
            state = self.recover_legacy(request, root, journal, state)
        try:
            if legacy and request["action"] == "activate":
                # Activation must not reuse the old operation's route table.
                edge = root / "edge/releases" / request["operation"]
                caddy = regular(edge / "caddy")[0]
                require(digest(caddy) == host["caddy_sha256"], "legacy Caddy changed")
                self.operation({**request, "action": "prepare", "caddy": base64.b64encode(caddy).decode()},
                               root, journal, state)
                state = json.loads(regular(journal)[0])
            return self.operation(request, root, journal, state)

        except Exception:
            self.cleanup_failed(request, root, journal)
            raise

    @staticmethod
    def validate_context(state, request):
        if request.get("recovery"):
            require(state and state["operation"] == request["operation"],
                    "recovery requires the original journal operation")
        if state and state["phase"] not in ("complete", "reverted"):
            require(state["operation"] == request["operation"], "another operation owns interrupted/prepared state")
            if request["action"] != "status" and ("rollback" in state or "transaction" in state):
                require(state.get("rollback", False) == request.get("rollback", False),
                        "prepared rollback intent differs")

        # Validate before entering cleanup handling: a rejected request
        # must never acquire ownership of an earlier prepared candidate.
        if state and state["operation"] == request["operation"]:
            require(state["version"] == request["version"]
                    and state["binding"] == digest(canonical(request["host"])), "operation context differs")

    @staticmethod
    def legacy_context(state):
        return bool(state and state["phase"] not in ("complete", "reverted")
                    and "transaction" not in state
                    and ("rollback" not in state or state.get("legacy_recovery") is True))

    def recover_legacy(self, request, root, journal, state):
        """Retire an owned old-worker attempt; never promote its reused edge."""
        op = ReleaseOperation(request, root, journal, state)
        phase = state.get("legacy_phase", state["phase"])
        require(phase in ("installing", "prepared", "promoting", "units_promoted", "retiring"),
                "unsupported legacy recovery phase")
        require(set(state["previous"]) == set(op.releases), "legacy selection differs")
        require(pointer(root / "edge/current") in (state["previous_edge"], "releases/" + op.edge.name)
                and all(pointer(root / "units" / unit / "current") in (old, "releases/" + op.version)
                        for unit, old in state["previous"].items()), "legacy pointers changed")
        if phase != "installing":
            self.verify(op.host, op.releases, op.edge)
            self.check_legacy_installed_state(op, state)
        self.restoration_inputs(op.host, root, state)
        # Validation failure above cannot enter generic candidate cleanup.
        # Persist the newly supplied intent before any process is changed.
        state.update(legacy_recovery=True, legacy_phase=phase,
                     rollback=request.get("rollback", False))
        self.record(root, journal, state)
        self.fault("legacy_recovery")
        self.restore(op.host, root, state)
        self.fault("legacy_restored")
        state["phase"] = "reverted"
        self.record(root, journal, state)
        return state

    def check_legacy_installed_state(self, op, state):
        """Original static journals have no derived-address evidence to check.

        Accept only the exact static configuration pinned by their host. Any
        executable-era evidence still requires all of its derived-state pins.
        """
        derived = ("rendered_edge_sha256", "installed_processes", "previous_processes")

        if any(field in state for field in derived):
            self.check_installed_state(op.host, op.releases, op.edge, state)

            return
        require(not op.host.get("executables") and not op.host.get("linux_formats")
                and all(unit.startswith("frontends.") for unit in op.releases),
                "legacy static journal cannot authorize executable recovery")
        config, mode = regular(op.edge / "config.json")
        require(config == canonical(op.host["edge"]) and mode == 0o644,
                "legacy static configuration differs from pinned host")

    def cleanup_failed(self, request, root, journal):
        """Release a rejected preparation only after its own candidates stop."""
        failed = json.loads(regular(journal)[0]) if journal.exists() else None

        if not failed or failed["phase"] not in ("installing", "prepared"):
            return
        self.validate_context(failed, request)

        if pointer(root / "edge/current") != failed["previous_edge"]:
            return

        if any(pointer(root / "units" / unit / "current") != previous
               for unit, previous in failed["previous"].items()):
            return
        # Cleanup failure keeps this nonterminal journal exclusively owned.
        self.stop_candidates(request["host"], root, failed)
        failed["phase"] = "reverted"
        self.record(root, journal, failed)

    def record(self, root, journal, state):
        atomic(journal, canonical(state))
        safe_directory(root / "operations", create=True, mode=0o700)
        atomic(root / "operations" / (state["operation"] + ".json"), canonical(state))

        if "transaction" in state:
            safe_directory(root / "transactions", create=True, mode=0o700)
            atomic(root / "transactions" / (state["transaction"] + ".json"), canonical(state))

        for unit in state["previous"]:
            atomic(root / "units" / unit / "journal.json", canonical(state))

    def operation(self, request, root, journal, state):
        op = ReleaseOperation(request, root, journal, state)

        if request["action"] == "prepare":
            return self.prepare(op)
        return self.activate(op)

    def prepare(self, op):
        """Recover interrupted intent before installing or starting a new candidate."""
        # Permanent conflicts must not reserve an operation that cannot
        # ever complete. Check them before publishing installation intent.
        for unit, target in op.releases.items():
            if target.exists():
                require(regular(target / "inventory.json")[0] == canonical(op.host["units"][unit]),
                        "release identifier already names different bytes")
                self.verify_release(op.host, unit, target)
            else:
                require(not op.request.get("rollback"), "rollback release is not installed")
        current_edge = pointer(op.root / "edge/current")

        if current_edge:
            self.check_template(op.host, op.root / "edge" / current_edge)

        if op.state and op.state["operation"] == op.identifier and op.state["phase"] not in ("complete", "reverted", "prepared"):
            if op.state["phase"] != "installing":
                self.restore(op.host, op.root, op.state)
            op.state["phase"] = "reverted"
            self.record(op.root, op.journal, op.state)

        if not (op.state and op.state["operation"] == op.identifier and op.state["phase"] in ("complete", "prepared")):
            self.begin_installation(op)

        excluded = {item["effective"]["address"] for item in op.state.get("previous_processes", {}).values()}

        for unit, target in op.releases.items():
            self.install_release(op, unit, target, excluded)

        if not op.edge.exists():
            self.install_edge(op)

        return self.finish_preparation(op)

    def begin_installation(self, op):
        """Pin previous release inputs before publishing any installation intent."""
        transaction = digest(canonical([op.identifier, os.urandom(16).hex()]))
        op.edge = op.root / "edge/releases" / transaction
        op.state = {"operation": op.identifier, "transaction": transaction,
                 "binding": op.binding, "version": op.version, "phase": "installing",
                 "rollback": op.request.get("rollback", False),
                 "previous": {u: pointer(op.root / "units" / u / "current") for u in op.releases},
                 "previous_edge": pointer(op.root / "edge/current")}
        op.state["previous_pins"] = {}
        op.state["previous_processes"] = {}

        for unit, previous in op.state["previous"].items():
            if previous:
                old = op.root / "units" / unit / previous
                tree = json.loads(regular(old / "inventory.json")[0])
                inventory(old / "assets", tree)
                op.state["previous_pins"][unit] = tree

                if (old / "process-request.json").exists():
                    requested = json.loads(regular(old / "process-request.json")[0])
                    effective = self.instances.check(old, requested)
                    op.state["previous_processes"][unit] = {"requested": requested, "effective": effective}
                else:
                    require(unit not in op.host.get("executables", {}),
                            "cannot replace a static unit with an executable in place")

        if op.state["previous_edge"]:
            old_edge = op.root / "edge" / op.state["previous_edge"]
            names = ("caddy", "config.json", "template.json")
            op.state["previous_edge_pins"] = {
                name: digest(regular(old_edge / name)[0]) for name in names if (old_edge / name).exists()}

        self.record(op.root, op.journal, op.state)
        self.fault("installing")

    def install_release(self, op, unit, target, excluded):
        """Copy one unit privately; an existing version must match its exact inputs."""
        if target.exists():
            self.verify_release(op.host, unit, target)

            if unit in op.host.get("executables", {}):
                excluded.add(self.instances.check(target, op.host["executables"][unit])["address"])

            return
        require(not op.request.get("rollback"), "rollback release is not installed")
        temporary = pathlib.Path(tempfile.mkdtemp(prefix=".stage-", dir=target.parent))
        os.chmod(temporary, 0o700)
        assets = temporary / "assets"
        assets.mkdir(mode=0o755)

        assets.chmod(0o755)

        for path, member in sorted(op.host["units"][unit].items()):
            destination = assets / path

            if member["kind"] == "directory":
                destination.mkdir(mode=0o755)
                destination.chmod(0o755)
            else:
                data = base64.b64decode(op.request["files"][unit + "/" + path], validate=True)
                require(len(data) == member["size"] and digest(data) == member["sha256"]
                        and member["mode"] == 0o644, "copied bytes differ")
                atomic(destination, data, 0o644)
        inventory(assets, op.host["units"][unit])

        atomic(temporary / "inventory.json", canonical(op.host["units"][unit]))

        if unit in op.host.get("executables", {}):
            self.instances.install(temporary, op.host["executables"][unit],
                                   op.request["backends"][unit], excluded)
        # Receipts stay root-only; public data must be traversable by Caddy.
        os.chmod(temporary, 0o755)
        os.rename(temporary, target)
        sync(target.parent)
        self.fault("installed:" + unit)

    def install_edge(self, op):
        """Keep shared route policy unchanged while materializing selected addresses."""
        temporary = pathlib.Path(tempfile.mkdtemp(prefix=".stage-", dir=op.edge.parent))
        if op.host["edge"] is not None:
            caddy = base64.b64decode(op.request["caddy"], validate=True)
            require(digest(caddy) == op.host["caddy_sha256"], "Caddy bytes differ")
            atomic(temporary / "caddy", caddy, 0o755)
        previous_edge = (op.root / "edge" / op.state["previous_edge"] / "config.json"
                         if op.state["previous_edge"] else None)
        previous = json.loads(regular(previous_edge)[0]) if previous_edge else None
        replacements = {requested["address"]: self.instances.check(op.releases[unit], requested)["address"]

                        for unit, requested in op.host.get("executables", {}).items()}
        rendered = render_edge(op.host["edge"], previous, replacements)
        atomic(temporary / "config.json", canonical(rendered), 0o644)
        atomic(temporary / "template.json", canonical(op.host["edge"]), 0o644)
        os.chmod(temporary, 0o755)
        os.rename(temporary, op.edge)
        sync(op.edge.parent)

    def finish_preparation(self, op):
        """Pin derived inputs before starting candidates or reporting preparation."""
        current_edge = pointer(op.root / "edge/current")

        if current_edge:
            self.check_template(op.host, op.root / "edge" / current_edge)
        self.verify(op.host, op.releases, op.edge)

        if "installed_processes" not in op.state:
            op.state["installed_processes"] = {
                unit: self.instances.check(op.releases[unit], requested)
                for unit, requested in op.host.get("executables", {}).items()}
            op.state["rendered_edge_sha256"] = digest(regular(op.edge / "config.json")[0])
            self.record(op.root, op.journal, op.state)
        self.check_installed_state(op.host, op.releases, op.edge, op.state)

        if op.state["phase"] != "complete":
            self.check_preserved_routes(op)
        # Durable intent already covers all candidate instances. A crash
        # before or during launch is recoverable without stopping an old one.
        op.state["observations"] = self.instances.start(op.host, op.releases)
        self.record(op.root, op.journal, op.state)
        self.fault("candidates_started")

        self.runtime.candidate(op.host, op.edge, op.releases)

        if op.state and op.state["operation"] == op.identifier and op.state["phase"] == "complete":
            return op.state
        op.state["phase"] = "prepared"
        self.record(op.root, op.journal, op.state)
        self.fault("prepared")

        return op.state

    def activate(self, op):
        """Only a matching prepared operation can begin promotion."""
        require(op.request["action"] == "activate" and op.state and op.state["operation"] == op.identifier
                and op.state["binding"] == op.binding, "no matching prepared operation")
        self.verify(op.host, op.releases, op.edge)
        self.check_installed_state(op.host, op.releases, op.edge, op.state)

        if op.state["phase"] == "complete":
            require(all(pointer(op.root / "units" / u / "current") == "releases/" + op.version for u in op.releases)
                    and pointer(op.root / "edge/current") == "releases/" + op.edge.name, "completed pointers differ")
            self.runtime.candidate(op.host, op.edge, op.releases)
            op.state["observations"] = self.instances.observe(op.host, op.releases, enable=True)
            self.runtime.active(op.host, op.releases)
            self.retire_previous(op.host, op.root, op.state)
            self.record(op.root, op.journal, op.state)

            return op.state

        if op.state["phase"] != "prepared":
            self.restore(op.host, op.root, op.state)
            op.state["phase"] = "reverted"
            self.record(op.root, op.journal, op.state)

            return op.state
        self.check_preserved_routes(op)
        op.state["observations"] = self.instances.start(op.host, op.releases)
        self.runtime.candidate(op.host, op.edge, op.releases)
        self.verify(op.host, op.releases, op.edge)
        self.check_preserved_routes(op)

        self.runtime.preflight(op.host)

        return self.promote(op)

    def promote(self, op):
        """Keep recovery ownership until new routes are ready and old instances retire."""
        op.state["phase"] = "promoting"
        self.record(op.root, op.journal, op.state)
        self.fault("before_units")
        try:
            for unit in op.releases:
                switch(op.root / "units" / unit / "current", "releases/" + op.version)
                self.fault("switched:" + unit)
            op.state["phase"] = "units_promoted"
            self.record(op.root, op.journal, op.state)

            self.fault("before_edge")
            switch(op.root / "edge/current", "releases/" + op.edge.name)
            self.fault("after_edge")
            self.runtime.restart(op.host)
            self.fault("after_restart")
            self.runtime.active(op.host, op.releases)
            op.state["observations"] = self.instances.observe(op.host, op.releases, enable=True)
            self.runtime.preflight(op.host)

            self.verify(op.host, op.releases, op.edge)
            self.check_installed_state(op.host, op.releases, op.edge, op.state)
            op.state["phase"] = "retiring"
            self.record(op.root, op.journal, op.state)
            self.fault("before_retirement")
            self.retire_previous(op.host, op.root, op.state)
            self.fault("after_retirement")
            op.state["phase"] = "complete"
            self.record(op.root, op.journal, op.state)
            self.fault("complete")
            # Per-unit journals are derived from the committed host journal.
            for unit in op.releases:
                atomic(op.root / "units" / unit / "journal.json", canonical(op.state))

            return op.state

        except Exception:
            self.restore(op.host, op.root, op.state)
            op.state["phase"] = "reverted"
            self.record(op.root, op.journal, op.state)
            raise


    def check_preserved_routes(self, op):
        """A rollback selects old binaries, never an old unrelated route table."""
        require(pointer(op.root / "edge/current") == op.state["previous_edge"]
                and all(pointer(op.root / "units" / unit / "current") == previous
                        for unit, previous in op.state["previous"].items()),
                "current release changed during preparation")
        previous = None

        if op.state["previous_edge"]:
            old = op.root / "edge" / op.state["previous_edge"]
            require(all(digest(regular(old / name)[0]) == pin
                        for name, pin in op.state["previous_edge_pins"].items()),
                    "previous edge changed during preparation")
            previous = json.loads(regular(old / "config.json")[0])
        replacements = {requested["address"]: self.instances.check(op.releases[unit], requested)["address"]

                        for unit, requested in op.host.get("executables", {}).items()}
        expected = render_edge(op.host["edge"], previous, replacements)
        require(regular(op.edge / "config.json")[0] == canonical(expected),
                "candidate changes an unselected route binding")

    def verify_release(self, host, unit, release):
        safe_directory(release)
        expected = {"assets", "inventory.json"}
        contract = host.get("executables", {}).get(unit)

        if contract:
            expected |= {"backend", "process.json", "process-request.json"}

        require(set(p.name for p in release.iterdir()) == expected, "unexpected release member")
        require(regular(release / "inventory.json")[0] == canonical(host["units"][unit]), "installed inventory differs")
        inventory(release / "assets", host["units"][unit])

        if contract:
            self.instances.check(release, contract)

    def check_template(self, host, edge):
        path = edge / "template.json"
        # Legacy static releases had no derived process addresses.
        template = regular(path if path.exists() else edge / "config.json")[0]
        require(template == canonical(host["edge"]), "release cannot change shared edge routes")

        return edge_addresses(host["edge"], json.loads(regular(edge / "config.json")[0]))

    def verify(self, host, releases, edge):
        for unit, release in releases.items():
            self.verify_release(host, unit, release)
        if host["edge"] is None:
            require(set(p.name for p in edge.iterdir()) == {"config.json", "template.json"},
                    "private transaction contains an edge runtime")
            for name in ("config.json", "template.json"):
                require(regular(edge / name) == (canonical(None), 0o644),
                        "private transaction routing metadata differs")
        else:
            require(digest(regular(edge / "caddy")[0]) == host["caddy_sha256"]
                    and regular(edge / "caddy")[1] == 0o755, "installed edge differs")
        addresses = self.check_template(host, edge)

        for unit, contract in host.get("executables", {}).items():
            if unit in host.get("routes", {}):
                require(addresses.get(contract["address"]) == self.instances.check(releases[unit], contract)["address"],
                        "edge does not route to selected executable instance")

    def check_installed_state(self, host, releases, edge, state):
        require(state.get("installed_processes", {}) == {
            unit: self.instances.check(releases[unit], requested)
            for unit, requested in host.get("executables", {}).items()},
            "installed process changed after preparation")
        require(state.get("rendered_edge_sha256") == digest(regular(edge / "config.json")[0]),
                "rendered edge changed after preparation")

    def stop_candidates(self, host, root, state):
        for unit, requested in host.get("executables", {}).items():
            if state["previous"][unit] != "releases/" + state["version"]:
                release = root / "units" / unit / "releases" / state["version"]

                if release.exists():
                    self.instances.stop(release, requested)

    def retire_previous(self, host, root, state):
        for unit, previous in state.get("previous_processes", {}).items():
            if state["previous"][unit] != "releases/" + state["version"]:
                self.instances.stop(root / "units" / unit / state["previous"][unit], previous["requested"])

    def restoration_inputs(self, host, root, state):
        """Check all prior release pins before recovery can change a process."""
        for unit, target in state["previous"].items():
            if target:
                old = root / "units" / unit / target
                inventory(old / "assets", state["previous_pins"][unit])

        if state["previous_edge"]:
            old_edge = root / "edge" / state["previous_edge"]
            require(all(digest(regular(old_edge / name)[0]) == pin
                        for name, pin in state["previous_edge_pins"].items()), "previous edge changed; recovery blocked")
        restored = copy.deepcopy(host)
        restored["executables"] = {}

        for unit, previous in state.get("previous_processes", {}).items():
            old = root / "units" / unit / state["previous"][unit]
            require(self.instances.check(old, previous["requested"]) == previous["effective"],
                    "previous process contract changed; recovery blocked")
            restored["executables"][unit] = previous["requested"]
        previous_releases = {unit: root / "units" / unit / target

                             for unit, target in state["previous"].items() if target}
        restored["units"] = state["previous_pins"]

        for unit, release in previous_releases.items():
            self.verify_release(restored, unit, release)

        return restored, previous_releases

    def restore(self, host, root, state):
        # A failed restoration retains exclusive ownership for an explicit retry.
        restored, previous_releases = self.restoration_inputs(host, root, state)
        # Old instances may have been retired just before interruption. Start
        # and observe them before pointing any route back at them.
        self.instances.start(restored, previous_releases)
        self.instances.observe(restored, previous_releases, enable=True)

        for unit, target in state["previous"].items():
            switch(root / "units" / unit / "current", target)
        switch(root / "edge/current", state["previous_edge"])

        if state["previous_edge"]:
            restored["edge"] = json.loads(regular(root / "edge" / state["previous_edge"] / "config.json")[0])
            self.runtime.restart(restored)
            self.runtime.active(restored, previous_releases)
        else:
            self.runtime.stop(host)
        self.stop_candidates(host, root, state)


if __name__ == "__main__":
    try:
        request = json.loads(sys.stdin.buffer.read(96 * 1024 * 1024 + 1))
        print(json.dumps(Machine(Linux()).run(request)))

    except Exception:
        print(json.dumps({"error": "host operation failed; retained journal requires inspection"}))
        sys.exit(1)
