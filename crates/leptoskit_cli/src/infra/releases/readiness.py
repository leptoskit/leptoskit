"""leptoskit-current-release-readiness-v1: observe one existing container."""

import http.client
import ipaddress
import json
import subprocess
import sys

request = json.load(sys.stdin)
container = request["container"]
port = request["port"]


def require(condition):
    if not condition:
        raise RuntimeError("current release readiness or configuration differs")


require(isinstance(container, str) and container and not container.startswith("-"))
require(type(port) is int and 1 <= port <= 65535)
active = subprocess.run(
    ["docker", "ps", "--filter", "label=service=" + request["service"],
     "--filter", "label=role=web", "--format", "{{.Names}}"],
    check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10,
)
require(active.stdout.decode().splitlines() == [container])
result = subprocess.run(
    ["docker", "inspect", container],
    check=True,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    timeout=10,
)
records = json.loads(result.stdout)
require(len(records) == 1)
record = records[0]
require(record["Name"] == "/" + container)
require(record["State"]["Running"] is True)
require(record["State"].get("Restarting", False) is False)
environment = dict(entry.split("=", 1) for entry in record["Config"]["Env"])
require(all(environment.get(name) == value for name, value in request["environment"].items()))
address = str(ipaddress.IPv4Address(record["NetworkSettings"]["Networks"]["kamal"]["IPAddress"]))
connection = http.client.HTTPConnection(address, port, timeout=5)
try:
    connection.request("GET", "/readyz")
    response = connection.getresponse()
    require(response.status == 200)
finally:
    connection.close()
print(json.dumps({"ready": True}))
