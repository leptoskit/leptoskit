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


before = snapshot()
(W / "before-update.json").write_text(json.dumps(before, indent=2))
descriptor = W / "website-v2.json"
text = descriptor.read_text()
d = json.loads(text)
run(
    cli,
    "verify",
    "approve-backend",
    "--descriptor",
    str(descriptor),
    "--key",
    str(W / "reviewer.key"),
    "--out",
    str(W / "website-v2-approval.json"),
)
operator = W / "website/infra/workspace-website"
(operator / "backend-authorization.json").write_text(
    json.dumps(
        dict(
            format_version=1,
            descriptor=text,
            approvals=[json.loads((W / "website-v2-approval.json").read_text())],
            deployment_policy="",
        )
    )
)

policy = json.loads((W / "website-managed-policy.json").read_text())
policy["sequence"] = 3
policy["releases"].append(
    dict(
        commit=d["recipe"]["commit"],
        inventory_sha256=d["inventory_sha256"],
        executable_sha256=d["executable_sha256"],
        backend=dict(
            descriptor_sha256=sha(descriptor), engine_image_id=d["engine_image_id"]
        ),
    )
)

(W / "website-policy-v2.json").write_text(json.dumps(policy))
run(
    cli,
    "verify",
    "sign",
    "--policy",
    str(W / "website-policy-v2.json"),
    "--key",
    str(W / "deployment.key"),
    "--out",
    str(operator / "backend-deployment-v2.json"),
)
(operator / "backend-deployment-v2.json").replace(operator / "backend-deployment.json")
releases = (W / "releases.toml").read_text()
start = releases.index("[applications.website]")
releases = releases[:start] + releases[start:].replace(
    'version="v1"', 'version="v2"'
).replace("website-v1.json", "website-v2.json").replace(
    sha(W / "website-v1.json"), sha(descriptor)
).replace(
    "setup=true", "setup=false"
)
(W / "update-releases.toml").write_text(releases)
plan = W / "update-plan.json"

run(cli, "workspace", "plan", "--releases", "update-releases.toml", "--out", str(plan))
out = run(cli, "workspace", "deploy", "--plan", str(plan), "--sha256", sha(plan))
assert "Revalidated current release v1" in out
after = snapshot()
assert after["accounts"] == before["accounts"]
assert after["website"]["image"] != before["website"]["image"]
assert after["website"]["container"] != before["website"]["container"]
assert after["website"]["history"] != before["website"]["history"]
(W / "after-update.json").write_text(json.dumps(after, indent=2))
run(cli, "workspace", "deploy", "--plan", str(plan), "--sha256", sha(plan))
assert snapshot() == after
print("INDEPENDENT UPDATE AND RETRY PASSED")
