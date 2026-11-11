"""Immutable application instances and derived, local-only edge bindings.

These inputs are checked by the caller before use. Port allocation is a local
observation, not authorization or proof of a running artifact. A race to claim
an allocated port fails the process adapter's socket-bound readiness check.
"""
import base64
import copy
import hashlib
import json
import re
import socket

try:
    Processes
except NameError:
    from process import Processes


def insist(condition, message):
    if not condition:
        raise ValueError(message)


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def effective_contract(requested, address):
    result = copy.deepcopy(requested)
    result["address"] = address
    result["environment"][result["address_env"]] = address
    Processes.validate(result)

    return result


def proxy_slots(node):
    """Only the generated proxy upstream dial field may be materialized."""
    if isinstance(node, dict):
        if node.get("handler") == "reverse_proxy":
            upstreams = node.get("upstreams")
            insist(isinstance(upstreams, list) and len(upstreams) == 1
                   and set(upstreams[0]) == {"dial"}, "unexpected proxy shape")

            yield upstreams[0]

        for child in node.values():
            yield from proxy_slots(child)
    elif isinstance(node, list):
        for child in node:
            yield from proxy_slots(child)


def edge_addresses(template, rendered):
    """Reject every template change except validated loopback destinations."""
    normalized = copy.deepcopy(rendered)
    before, after = list(proxy_slots(template)), list(proxy_slots(normalized))
    insist(len(before) == len(after), "edge proxy membership changed")
    addresses = {}

    for source, destination in zip(before, after):
        address = destination["dial"]

        match = re.fullmatch(r"127\.0\.0\.1:([0-9]+)", address)
        insist(match and 1024 <= int(match[1]) <= 65535
               and int(match[1]) not in (8088, 18088), "invalid effective application port")
        original = source["dial"]
        insist(original not in addresses or addresses[original] == address,
               "one declared address has conflicting effective bindings")
        addresses[original] = address
        destination["dial"] = original
    insist(normalized == template, "edge changed beyond application address bindings")

    return addresses


def render_edge(template, previous, replacements):
    result = copy.deepcopy(template)
    addresses = edge_addresses(template, previous) if previous is not None else {}
    addresses.update(replacements)

    for slot in proxy_slots(result):
        slot["dial"] = addresses.get(slot["dial"], slot["dial"])
    edge_addresses(template, result)

    return result


class Instances:
    def __init__(self, adapter, read, write):
        self.adapter, self.read, self.write = adapter, read, write

    @staticmethod
    def validate(host):
        # Validate cross-field ownership before any release or unit is written.
        contracts = host.get("executables", {})
        insist(isinstance(contracts, dict) and set(contracts) <= set(host["units"]),
               "executable declarations must belong to selected units")
        addresses = set()

        for unit, contract in contracts.items():
            Processes.validate(contract)
            insist(contract["address"] not in addresses, "selected application addresses collide")
            addresses.add(contract["address"])

            if unit in host.get("routes", {}):
                mount = host["routes"][unit]["mount"]
                insist(all(contract[key].startswith(mount) for key in ("health", "ready")),
                       "health/readiness endpoints are outside the public mount")
                insist(any(slot["dial"] == contract["address"] for slot in proxy_slots(host["edge"])),
                       "application route has no declared upstream")

        return contracts

    @staticmethod
    def allocate(excluded):
        for _ in range(32):
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                port = listener.getsockname()[1]
            address = f"127.0.0.1:{port}"
            if port not in (8088, 18088) and address not in excluded:
                return address
        raise ValueError("no candidate application port available")

    def install(self, release, requested, payload, excluded):
        Processes.validate(requested)
        data = base64.b64decode(payload, validate=True)
        pin = requested["backend"]
        insist(len(data) == pin["size"] and hashlib.sha256(data).hexdigest() == pin["sha256"],
               "copied executable differs")
        effective = effective_contract(requested, self.allocate(excluded))
        excluded.add(effective["address"])
        self.write(release / "backend", data, 0o755)
        self.write(release / "process-request.json", encoded(requested))

        self.write(release / "process.json", encoded(effective))

    def check(self, release, requested):
        Processes.validate(requested)
        insist(self.read(release / "process-request.json")[0] == encoded(requested),
               "installed process request differs")
        effective = json.loads(self.read(release / "process.json")[0])
        insist(effective == effective_contract(requested, effective["address"]),
               "installed process configuration differs")
        data, mode = self.read(release / "backend")
        pin = requested["backend"]
        insist(mode == pin["mode"] and len(data) == pin["size"]
               and hashlib.sha256(data).hexdigest() == pin["sha256"], "installed executable differs")
        insist(self.read(release / "process-request.json")[1] == 0o600
               and self.read(release / "process.json")[1] == 0o600, "process receipt mode differs")

        self.adapter.check(release, effective)

        return effective

    def start(self, host, releases):
        observations = {}

        for unit, requested in host.get("executables", {}).items():
            release = releases[unit]
            observations[unit] = self.adapter.start(release, self.check(release, requested), self.write)

        return observations

    def observe(self, host, releases, enable=False):
        observations = {}

        for unit, requested in host.get("executables", {}).items():
            release = releases[unit]
            method = self.adapter.enable if enable else self.adapter.observe
            observations[unit] = method(release, self.check(release, requested))

        return observations

    def stop(self, release, requested):
        self.adapter.stop_if_installed(release, self.check(release, requested))
