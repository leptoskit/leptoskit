import hashlib,json,os,subprocess,shlex
from pathlib import Path
E=Path(__file__).parent;W=E/'journey';cli=os.environ['CARGO_TARGET_DIR']+'/debug/leptoskit';ssh=str(E/'bin/ssh')
env=dict(os.environ,PATH=str(E/'bin')+':'+os.environ['PATH'],GEM_HOME=str(E/'gems'),KAMAL_REGISTRY_PASSWORD='local-registry',ACCOUNTS_ADMIN='local-accounts-admin',ACCOUNTS_APP='local-accounts-app',WEBSITE_ADMIN='local-website-admin',WEBSITE_APP='local-website-app')
def run(*args,env=env,ok=True):
 r=subprocess.run(args,cwd=W,env=env,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);print(r.stdout,flush=True);assert (r.returncode==0)==ok,(args,r.returncode);return r.stdout

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def snapshot():
 result={}
 for app in ['accounts','website']:
  name='workspace-'+app
  fmt='{"container":{{json .Id}},"image":{{json .Image}},"started":{{json .State.StartedAt}}}'
  raw=run(ssh,'root@127.0.0.1',f"docker inspect --format {shlex.quote(fmt)} $(docker ps -q --filter label=service={name} --filter label=role=web)")
  c=json.loads(raw)
  history=run(ssh,'root@127.0.0.1',f'cat /var/lib/leptoskit/{name}/control/releases.json')
  result[app]=dict(**c,history=hashlib.sha256(history.encode()).hexdigest())
 return result
plan=W/'initial-plan.json'
def deploy(ok=True,changes=None):return run(cli,'workspace','deploy','--plan',str(plan),'--sha256',sha(plan),env=env if changes is None else dict(env,**changes),ok=ok)

before=snapshot()
run(ssh,'root@127.0.0.1','docker stop workspace-website-web-v1')
try:
 out=deploy(False);assert 'Revalidated current release v1' in out
 journal=W/'.leptoskit/workspace'/(sha(plan)+'.json');value=json.loads(journal.read_text())
 assert value['applications']['accounts']['state']=='command_completed'
 assert value['applications']['website']['state']=='failed'
 (W/'partial-journal.json').write_bytes(journal.read_bytes())
finally:run(ssh,'root@127.0.0.1','docker start workspace-website-web-v1')
assert 'Revalidated current release' in deploy()
assert snapshot()['accounts']==before['accounts']
(W/'retried-journal.json').write_bytes(journal.read_bytes())
print('DURABLE PARTIAL FAILURE AND EXPLICIT RETRY PASSED')
