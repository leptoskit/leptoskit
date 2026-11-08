import hashlib, json, os, subprocess, shlex
from pathlib import Path

E = Path(__file__).parent
W = E / "journey"
cli = os.environ["CARGO_TARGET_DIR"] + "/debug/leptoskit"
ssh = str(E / "bin/ssh")
env = dict(
    os.environ,
    PATH=str(E / "bin") + ":" + os.environ["PATH"],
    GEM_HOME=str(E / "gems"),
    KAMAL_REGISTRY_PASSWORD="local-registry",
    ACCOUNTS_ADMIN="local-accounts-admin",
    ACCOUNTS_APP="local-accounts-app",
    WEBSITE_ADMIN="local-website-admin",
    WEBSITE_APP="local-website-app",
)


def run(*args, env=env, ok=True):
    r = subprocess.run(
        args,
        cwd=W,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    print(r.stdout, flush=True)
    assert (r.returncode == 0) == ok, (args, r.returncode)
    return r.stdout


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def snapshot():
    result = {}
    for app in ["accounts", "website"]:
        name = "workspace-" + app
        fmt = '{"container":{{json .Id}},"image":{{json .Image}},"started":{{json .State.StartedAt}}}'
        raw = run(
            ssh,
            "root@127.0.0.1",
            f"docker inspect --format {shlex.quote(fmt)} $(docker ps -q --filter label=service={name} --filter label=role=web)",
        )
        c = json.loads(raw)
        history = run(
            ssh,
            "root@127.0.0.1",
            f"cat /var/lib/leptoskit/{name}/control/releases.json",
        )
        result[app] = dict(**c, history=hashlib.sha256(history.encode()).hexdigest())

    return result


plan = W / "initial-plan.json"


def deploy(ok=True, changes=None):
    return run(
        cli,
        "workspace",
        "deploy",
        "--plan",
        str(plan),
        "--sha256",
        sha(plan),
        env=env if changes is None else dict(env, **changes),
        ok=ok,
    )


import time

before = snapshot()


def write_remote(path, value):
    result = subprocess.run(
        [ssh, "root@127.0.0.1", "cat > " + path],
        input=value if isinstance(value, str) else json.dumps(value),
        text=True,
        capture_output=True,
    )
    assert result.returncode == 0


base = "/var/lib/leptoskit/workspace-accounts/control/"
for name, field, value, expected in [
    ("releases.json", "configuration_sha256", None, "configuration"),
    ("operation.json", "phase", "preparing", "unfinished"),
]:
    original = run(ssh, "root@127.0.0.1", "cat " + base + name)
    changed = json.loads(original)
    if name == "releases.json":
        changed[-1].pop(field)
    else:
        changed[field] = value
    try:
        write_remote(base + name, changed)
        assert expected in deploy(False)
    finally:
        write_remote(base + name, original)
    assert snapshot() == before
operator = W / "accounts/infra/workspace-accounts"
original = json.loads((W / "accounts-managed-policy.json").read_text())


def policy(seq, kind):
    p = json.loads(json.dumps(original))
    p["sequence"] = seq

    if kind == "expired":
        p["issued_at"] = int(time.time()) - 7200
        p["expires_at"] = int(time.time()) - 60

    if kind == "revoked":
        replacement = json.loads((W / "website-v2.json").read_text())
        p["releases"] = [
            dict(
                commit=replacement["recipe"]["commit"],
                inventory_sha256=replacement["inventory_sha256"],
                executable_sha256=replacement["executable_sha256"],
                backend=dict(
                    descriptor_sha256=sha(W / "website-v2.json"),
                    engine_image_id=replacement["engine_image_id"],
                ),
            )
        ]
    source = W / f"accounts-policy-{seq}.json"
    source.write_text(json.dumps(p))
    target = W / f"accounts-envelope-{seq}.json"
    run(
        cli,
        "verify",
        "sign",
        "--policy",
        str(source),
        "--key",
        str(W / "deployment.key"),
        "--out",
        str(target),
    )
    (operator / "backend-deployment.json").write_bytes(target.read_bytes())


for seq, kind in [(7, "expired"), (9, "revoked")]:

    try:
        policy(seq, kind)
        out = deploy(False)
        assert any(word in out for word in ["expired", "policy", "authorize"])
    finally:
        policy(seq + 1, "valid")
    assert snapshot() == before
    assert "Revalidated current release" in deploy()
print("LEGACY HISTORY, INCOMPLETE PHASE, EXPIRY AND REVOCATION PASSED")
