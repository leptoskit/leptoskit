import hashlib, json, os, subprocess
from pathlib import Path

E = Path(__file__).parent
W = E / "journey"
root = Path.cwd()
cli = os.environ["CARGO_TARGET_DIR"] + "/debug/leptoskit"


def run(*args, env=None, input=None):
    r = subprocess.run(
        args,
        env=env,
        input=input,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    print(r.stdout, flush=True)
    assert r.returncode == 0, args

    return r.stdout.strip()


env = dict(
    os.environ,
    GIT_CONFIG_COUNT="1",
    GIT_CONFIG_KEY_0=f"url.file://{root}.insteadOf",
    GIT_CONFIG_VALUE_0="https://github.com/leptoskit/leptoskit",
)
index = dict(env, GIT_INDEX_FILE=str(root / ".git/workspace-v2-fixture-index"))
recipe = json.loads((W / "recipe.json").read_text())
run("git", "read-tree", recipe["commit"], env=index)
source = (
    (root / "docs/agents/workspace-release-fixture/src/main.rs")
    .read_text()
    .replace('"ready\\n"', '"ready-v2\\n"')
)
assert "ready-v2" in source
blob = run("git", "hash-object", "-w", "--stdin", input=source)
run("git", "update-index", "--cacheinfo", "100644," + blob + ",src/main.rs", env=index)

tree = run("git", "write-tree", env=index)
recipe["commit"] = run(
    "git", "commit-tree", "-S", tree, "-m", "test: changed website serving fixture"
)
p = W / "recipe-v2.json"
p.write_text(json.dumps(recipe))
out = W / "build-v2"
run(
    cli,
    "verify",
    "build",
    "--recipe",
    str(p),
    "--out",
    str(out),
    "--engine",
    "podman",
    env=env,
)
pin = hashlib.sha256((out / "inventory.json").read_bytes()).hexdigest()
desc = json.loads((W / "website-v1.json").read_text())
images = json.loads(run("podman", "image", "inspect", "alpine:3.21"))
runtime = images[0]["RepoDigests"][0]
runtime_pin = hashlib.sha256(
    json.dumps(images[0]["Config"], sort_keys=True, separators=(",", ":")).encode()
).hexdigest()
run(
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
    "127.0.0.1:5000/website:v2",
    "--out",
    str(W / "website-v2.json"),
    env=env,
)
run("podman", "push", "--tls-verify=false", "127.0.0.1:5000/website:v2")
