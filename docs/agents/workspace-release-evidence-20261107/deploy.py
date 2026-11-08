import hashlib, json, os, shutil, subprocess, tomllib
from pathlib import Path

E = Path(__file__).parent
W = E / "journey"
ROOT = Path.cwd()
CLI = os.environ["CARGO_TARGET_DIR"] + "/debug/leptoskit"


def run(*args, cwd=W, env=None, ok=True):
    r = subprocess.run(
        args,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    print(r.stdout, flush=True)

    if (r.returncode == 0) != ok:
        raise RuntimeError(args)

    return r.stdout.strip()


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def tool(name, body):
    p = E / "bin" / name
    p.write_text(body)
    p.chmod(0o700)


env = dict(
    os.environ, PATH=str(E / "bin") + ":" + os.environ["PATH"], GEM_HOME=str(E / "gems")
)
# These throwaway, local-only values are not production credentials.
env.update(
    KAMAL_REGISTRY_PASSWORD="local-registry",
    ACCOUNTS_ADMIN="local-accounts-admin",
    ACCOUNTS_APP="local-accounts-app",
    WEBSITE_ADMIN="local-website-admin",
    WEBSITE_APP="local-website-app",
)
ssh = str(E / "bin/ssh")
port = json.loads(run("podman", "machine", "inspect", "workspace-release-20261107"))[0][
    "SSHConfig"
]["Port"]
tool(
    "tofu",
    """#!/usr/bin/env python3
import json,pathlib,sys,tomllib
c=tomllib.loads((pathlib.Path.cwd().parents[1]/'leptoskit.toml').read_text())['infra']
a=c['name'].split('-')[-1]
v={'server_ips':['127.0.0.1'],'server_ip':'127.0.0.1','private_ip':'127.0.0.'+('2' if a=='accounts' else '3'),'app_url':'https://'+c['domain'],'updates_url':'https://'+c['updates_domain'],'updates_bucket':c['updates_bucket'],'backup_bucket':c['backup_bucket'],'backup_endpoint':'https://'+c['account_id']+'.r2.cloudflarestorage.com','s3_endpoint':'https://'+c['account_id']+'.r2.cloudflarestorage.com','tunnel_token':'local-provider-fixture'}
assert sys.argv[1]=='output', 'provider fixture permits output only'
value=v[sys.argv[-1]]
print(json.dumps(value) if '-json' in sys.argv else value)
""",
)
tool(
    "kamal",
    f"""#!/usr/bin/env python3
import json,os,pathlib,sys
p=pathlib.Path('release.yml'); c=json.loads(p.read_text())
c['ssh']={{'port':{port},'keys':['/Users/mini02/.local/share/containers/podman/machine/machine'],'config':['{E}/ssh_config']}}
c['builder']['arch']='arm64'
p.write_text(json.dumps(c))
os.environ.setdefault('VERSION','local-fixture')
args=sys.argv[1:]
if args[:2]==['registry','login']: args+=['--skip-local']
os.execv('{E}/gems/bin/kamal',['kamal',*args])
""",
)
# A provider cloud-init stub and idle tunnel containers are explicit local fixtures.
run(
    ssh,
    "root@127.0.0.1",
    "printf '#!/bin/sh\nexit 0\n' > /var/opt/workspace-release/docker/cloud-init; chmod 755 /var/opt/workspace-release/docker/cloud-init",
)

for app in ["accounts", "website"]:
    d = W / app

    if not d.exists():
        shutil.copytree(ROOT / "docs/agents/workspace-release-fixture", d)
    name = "workspace-" + app
    config = (
        ROOT / "docs/agents/workspace-release-fixture/leptoskit.toml"
    ).read_text() + f'\n[infra]\nname="{name}"\ndomain="{app}.example.test"\nlocation="hel1"\nserver_type="cx23"\nssh_key_ids=[123]\nssh_cidrs=["127.0.0.1/32"]\naccount_id="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"\nzone_id="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"\nupdates_domain="updates-{app}.example.test"\nupdates_bucket="{name}-releases"\nbackup_bucket="{name}-backups"\nimage="127.0.0.1:5000/{app}"\nregistry="127.0.0.1:5000"\nregistry_username="fixture"\n[infra.backups]\nenabled=false\n'
    (d / "leptoskit.toml").write_text(config)
    run(
        ssh,
        "root@127.0.0.1",
        f"docker inspect {name}-tunnel >/dev/null 2>&1 || docker run -d --name {name}-tunnel alpine:3.21 sleep infinity",
    )
    # Actual Postgres container and table, independently owned by each app.
    run(
        ssh,
        "root@127.0.0.1",
        f"docker inspect {name}-db >/dev/null 2>&1 || docker run -d --name {name}-db -e POSTGRES_USER=leptoskit -e POSTGRES_DB=leptoskit -e POSTGRES_HOST_AUTH_METHOD=trust postgres:17",
    )
    run(
        ssh,
        "root@127.0.0.1",
        f"for i in $(seq 1 60); do docker exec {name}-db pg_isready -U leptoskit >/dev/null && break; sleep 1; done; docker exec {name}-db psql -U leptoskit -d leptoskit -c 'CREATE TABLE IF NOT EXISTS _leptoskit_migrations (name text, checksum text);'",
    )

(W / "leptoskit.workspace.toml").write_text(
    'format_version=1\n[applications.accounts]\npath="accounts"\n[applications.website]\npath="website"\ndepends-on=["accounts"]\n'
)
releases = "format_version=1\n"

for app in ["accounts", "website"]:
    releases += f'[applications.{app}]\nversion="v1"\ndescriptor="{app}-v1.json"\ndescriptor-sha256="{sha(W/(app+"-v1.json"))}"\nsetup=true\n[applications.{app}.environment]\nPOSTGRES_PASSWORD="{app.upper()}_ADMIN"\nDATABASE_PASSWORD="{app.upper()}_APP"\n'
(W / "releases.toml").write_text(releases)
plan = W / "initial-plan.json"

if not plan.exists():
    run(
        CLI,
        "workspace",
        "plan",
        "--releases",
        "releases.toml",
        "--out",
        str(plan),
        env=env,
    )
run(CLI, "workspace", "deploy", "--plan", str(plan), "--sha256", sha(plan), env=env)
print("INITIAL TWO APP DEPLOY PASSED")
