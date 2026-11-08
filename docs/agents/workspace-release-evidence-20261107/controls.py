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


before = snapshot()
(W / "initial-snapshot.json").write_text(json.dumps(before, indent=2))
assert "Revalidated current release" in deploy()
assert snapshot() == before
out = deploy(False, dict(ACCOUNTS_APP="rotated-local-credential"))
assert "remote operation failed" in out
assert snapshot() == before

for path in [
    W / "accounts-v1.json",
    W / "accounts/leptoskit.toml",
    W / "leptoskit.workspace.toml",
]:
    original = path.read_bytes()
    try:
        path.write_bytes(
            original.replace(b"accounts.example.test", b"changed.example.test")
            if path.name == "leptoskit.toml"
            else original + b" "
        )

        out = deploy(False)
        assert any(word in out for word in ["changed", "differs", "pin", "hash"])

    finally:
        path.write_bytes(original)
    assert snapshot() == before
policy = W / "accounts/infra/workspace-accounts/backend-deployment.json"
original = policy.read_bytes()
try:
    policy.write_text("{}")
    assert "policy" in deploy(False)
finally:
    policy.write_bytes(original)
assert snapshot() == before
# Stopped provider must fail before the consumer is invoked.
run(ssh, "root@127.0.0.1", "docker stop workspace-accounts-web-v1")
try:
    assert "failed" in deploy(False)
finally:
    run(ssh, "root@127.0.0.1", "docker start workspace-accounts-web-v1")
# A consumer failure persists the successfully revalidated provider in progress.
run(ssh, "root@127.0.0.1", "docker stop workspace-website-web-v1")
try:
    assert "failed" in deploy(False)
finally:
    run(ssh, "root@127.0.0.1", "docker start workspace-website-web-v1")
assert "Revalidated current release" in deploy()
print("RETRY AND NEGATIVE CONTROLS PASSED")
