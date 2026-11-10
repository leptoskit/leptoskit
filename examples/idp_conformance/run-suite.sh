#!/usr/bin/env bash
# Runs the OpenID Foundation's Basic OP and Config OP plans against the
# conformance test server, and fails on any module that fails, warns, or is
# skipped unless expected-failures.json or expected-skips.json lists it.
#
#   run-suite.sh <suite checkout, built> <test server binary> <results directory>
#
# The suite must already be built (target/fapi-test-suite.jar). Needs docker
# compose (or podman with DOCKER_HOST pointing at it), openssl and python3.
set -euo pipefail

suite="$(cd "$1" && pwd)"
binary="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
results="$3"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
mkdir -p "$results"
results="$(cd "$results" && pwd)"

# localhost.emobix.co.uk resolves to 127.0.0.1; inside the suite's container,
# a redirector forwards it to the suite's nginx.
suite_url="https://localhost.emobix.co.uk:8443"
issuer="https://idp.test:9443"
compose=(docker compose --project-name leptoskit-conformance -f "$suite/docker-compose.yml" -f "$here/compose.yml")

server_pid=""
cleanup() {
	if [ -n "$server_pid" ]; then
		kill "$server_pid" 2>/dev/null || true
	fi
	"${compose[@]}" logs server >"$results/suite.log" 2>&1 || true
	"${compose[@]}" down --volumes >/dev/null 2>&1 || true
	rm -rf "$work"
}
trap cleanup EXIT

# Keys made for this run only: the server's certificate and its RSA key.
openssl req -x509 -nodes -newkey rsa:2048 -days 2 -subj "/CN=idp.test" \
	-addext "subjectAltName=DNS:idp.test" \
	-keyout "$work/tls.key" -out "$work/tls.crt" 2>/dev/null
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$work/rsa.pem" 2>/dev/null

echo "Starting the conformance suite"
"${compose[@]}" up --detach --build --quiet-pull >"$results/compose.log" 2>&1
for _ in $(seq 1 120); do
	if curl --silent --insecure --fail --output /dev/null "$suite_url/api/server"; then
		break
	fi
	sleep 2
done
curl --silent --insecure --fail --output /dev/null "$suite_url/api/server"

echo "Starting the test server at $issuer"
IDP_ISSUER="$issuer" \
	IDP_LISTEN="0.0.0.0:9443" \
	IDP_TLS_CERT="$work/tls.crt" \
	IDP_TLS_KEY="$work/tls.key" \
	IDP_RSA_KEY="$(cat "$work/rsa.pem")" \
	CONFORMANCE_SUITE="$suite_url" \
	CONFORMANCE_CONFIG="$work/config.json" \
	"$binary" >"$results/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 60); do
	if [ -s "$work/config.json" ] && grep -q listening "$results/server.log"; then
		break
	fi
	sleep 1
done
grep -q listening "$results/server.log"

python3 -m venv "$work/venv"
"$work/venv/bin/pip" install --quiet --requirement "$suite/scripts/requirements.txt"

echo "Running Basic OP and Config OP"
cd "$suite"
CONFORMANCE_SERVER="$suite_url/" \
	CONFORMANCE_SERVER_MTLS="$suite_url/" \
	CONFORMANCE_DEV_MODE=1 \
	"$work/venv/bin/python3" scripts/run-test-plan.py \
	--export-dir "$results" \
	--expected-failures-file "$here/expected-failures.json" \
	--expected-skips-file "$here/expected-skips.json" \
	"oidcc-basic-certification-test-plan[server_metadata=discovery][client_registration=static_client]" "$work/config.json" \
	"oidcc-config-certification-test-plan" "$work/config.json"
