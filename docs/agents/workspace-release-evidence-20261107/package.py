import hashlib, json, os, subprocess, sys
from pathlib import Path
root=Path.cwd()
evidence=Path(__file__).parent
cli=os.environ['CARGO_TARGET_DIR']+'/debug/leptoskit'
def run(*args, env=None):
    result=subprocess.run(args,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    print(result.stdout,flush=True)
    if result.returncode: raise RuntimeError(args)
    return result.stdout.strip()
def digest(b): return hashlib.sha256(b).hexdigest()
tree=run('git','rev-parse','HEAD:docs/agents/workspace-release-fixture')
commit=run('git','commit-tree','-S',tree,'-m','test: local workspace serving fixture')
env=dict(os.environ,GIT_CONFIG_COUNT='1',GIT_CONFIG_KEY_0=f'url.file://{root}.insteadOf',GIT_CONFIG_VALUE_0='https://github.com/leptoskit/leptoskit')
images=json.loads(run('podman','image','inspect','rust:1.85-alpine','alpine:3.21'))
builder,runtime=[v['RepoDigests'][0] for v in images]
runtime_pin=digest(json.dumps(images[1]['Config'],sort_keys=True,separators=(',',':')).encode())
work=evidence/'journey';work.mkdir(exist_ok=True)
recipe=work/'recipe.json'
recipe.write_text(json.dumps(dict(repository='https://github.com/leptoskit/leptoskit',commit=commit,builder_image=builder,target='aarch64-unknown-linux-musl',package='verified-backend',binary='verified-backend',features=[],no_default_features=False,rustflags=[])))
build=work/'build'
run(cli,'verify','build','--recipe',str(recipe),'--out',str(build),'--engine','podman',env=env)
pin=digest((build/'inventory.json').read_bytes())
common=['--bundle',str(build/'bundle'),'--manifest',str(build/'inventory.json'),'--sha256',pin,'--engine','podman']
for app in ['accounts','website']:
    descriptor=work/(app+'-v1.json')
    tag='127.0.0.1:5000/'+app+':v1'
    run(cli,'infra','build-verified',*common,'--runtime-image',runtime,'--runtime-config-sha256',runtime_pin,'--tag',tag,'--out',str(descriptor),env=env)
    run('podman','push','--tls-verify=false',tag)
    print(app,digest(descriptor.read_bytes()),flush=True)
