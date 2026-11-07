import hashlib,json,os,subprocess,time
from pathlib import Path
E=Path(__file__).parent;W=E/'journey';root=Path.cwd();cli=os.environ['CARGO_TARGET_DIR']+'/debug/leptoskit'
env=dict(os.environ,GIT_CONFIG_COUNT='1',GIT_CONFIG_KEY_0=f'url.file://{root}.insteadOf',GIT_CONFIG_VALUE_0='https://github.com/leptoskit/leptoskit')
def run(*args):
 r=subprocess.run(args,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);print(r.stdout,flush=True);assert r.returncode==0,args;return r.stdout
images=json.loads(run('podman','image','inspect','alpine:3.21'));runtime=images[0]['RepoDigests'][0];runtime_pin=hashlib.sha256(json.dumps(images[0]['Config'],sort_keys=True,separators=(',',':')).encode()).hexdigest()
for app,version in [('accounts','v1'),('website','v1'),('website','v2')]:
 out=W/('build-v2' if version=='v2' else 'build');pin=hashlib.sha256((out/'inventory.json').read_bytes()).hexdigest();desc=W/f'{app}-{version}.json'
 desc.rename(W/f'{app}-{version}-before-service.json')
 tag=f'127.0.0.1:5000/{app}:{version}'
 run(cli,'infra','build-verified','--bundle',str(out/'bundle'),'--manifest',str(out/'inventory.json'),'--sha256',pin,'--engine','podman','--runtime-image',runtime,'--runtime-config-sha256',runtime_pin,'--tag',tag,'--service','workspace-'+app,'--out',str(desc))
 run('podman','push','--tls-verify=false',tag)
for app in ['accounts','website']:
 desc=W/(app+'-v1.json');text=desc.read_text();d=json.loads(text)
 approval=W/(app+'-managed-approval.json')
 if not approval.exists(): run(cli,'verify','approve-backend','--descriptor',str(desc),'--key',str(W/'reviewer.key'),'--out',str(approval))
 operator=W/app/'infra'/('workspace-'+app)
 (operator/'backend-authorization.json').write_text(json.dumps(dict(format_version=1,descriptor=text,approvals=[json.loads(approval.read_text())],deployment_policy='')))
 policy=json.loads((W/(app+'-policy.json')).read_text());policy['sequence']=2;policy['releases']=[dict(commit=d['recipe']['commit'],inventory_sha256=d['inventory_sha256'],executable_sha256=d['executable_sha256'],backend=dict(descriptor_sha256=hashlib.sha256(text.encode()).hexdigest(),engine_image_id=d['engine_image_id']))]
 policyfile=W/(app+'-managed-policy.json');policyfile.write_text(json.dumps(policy));run(cli,'verify','sign','--policy',str(policyfile),'--key',str(W/'deployment.key'),'--out',str(operator/'backend-deployment-managed.json'));(operator/'backend-deployment-managed.json').replace(operator/'backend-deployment.json')
(W/'initial-plan.json').rename(W/'initial-plan-before-service.json')
