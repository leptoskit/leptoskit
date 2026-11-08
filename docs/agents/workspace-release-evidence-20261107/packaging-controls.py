import hashlib, json, os, subprocess, time
from pathlib import Path

E = Path(__file__).parent
W = E / "journey"
root = Path.cwd()
cli = os.environ["CARGO_TARGET_DIR"] + "/debug/leptoskit"
env = dict(
    os.environ,
    GIT_CONFIG_COUNT="1",
    GIT_CONFIG_KEY_0=f"url.file://{root}.insteadOf",
    GIT_CONFIG_VALUE_0="https://github.com/leptoskit/leptoskit",
)


def run(*args):
    r = subprocess.run(
        args, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT
    )
    print(r.stdout, flush=True)
    assert r.returncode == 0, args
    return r.stdout


images = json.loads(run("podman", "image", "inspect", "alpine:3.21"))
runtime = images[0]["RepoDigests"][0]
runtime_pin = hashlib.sha256(
    json.dumps(images[0]["Config"], sort_keys=True, separators=(",", ":")).encode()
).hexdigest()

out = W / "build"
pin = hashlib.sha256((out / "inventory.json").read_bytes()).hexdigest()
base = [
    cli,
    "infra",
    "build-verified",
    "--bundle",
    str(out / "bundle"),
    "--manifest",
    str(out / "inventory.json"),
    "--sha256",
    pin,
    "--engine",
    "podman",
    "--runtime-image",
    runtime,
    "--runtime-config-sha256",
    runtime_pin,
    "--tag",
    "127.0.0.1:5000/accounts:v1",
    "--reuse-existing",
]
for name, extra, expected in [
    ("wrong-service", ["--service", "different-service"], "configuration"),
    (
        "incompatible-rpc",
        [
            "--service",
            "workspace-accounts",
            "--rpc-baseline",
            str(root / "crates/leptoskit_cli/tests/fixtures/fns_spec.json"),
            "--rpc-baseline-sha256",
            hashlib.sha256(
                (
                    root / "crates/leptoskit_cli/tests/fixtures/fns_spec.json"
                ).read_bytes()
            ).hexdigest(),
        ],
        "RPC",
    ),
]:
    target = W / (name + "-must-not-exist.json")
    result = subprocess.run(
        base + extra + ["--out", str(target)],
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    print(result.stdout, flush=True)
    assert result.returncode != 0
    assert not target.exists()
    assert expected.lower() in result.stdout.lower()
run(
    *base,
    "--service",
    "workspace-accounts",
    "--out",
    str(W / "correct-service-reuse.json"),
)
print("REAL LABEL REUSE AND INCOMPATIBLE RPC CONTROLS PASSED")
