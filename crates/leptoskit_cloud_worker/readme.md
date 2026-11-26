# leptoskit_cloud_worker

<!-- {=cloudWorkerSummary} -->

`LeptosKit` Cloud execution worker (`leptoskit-cloud-worker`) supporting the `build` role (with the built-in `build-app` OCI builder and optional store publishing) and the `cluster` role (Kubernetes rollouts, shared Postgres provisioning, periodic status and runtime log reporting, and `--reconcile-on-start` recovery).

<!-- {/cloudWorkerSummary} -->

It pairs with a control plane, asks it for jobs, runs one at a time under a lease, batches job logs, and reports outcomes, environment status, and runtime logs back to the control plane. When `--build-command` is omitted, it defaults to its built-in `leptoskit-cloud-worker build-app` OCI builder (supporting persistent `--build-cache-dir` and optional `--build-sandbox` isolation wrappers). Once installed beside `lk` or on `PATH`, you can also run it via `lk cloud worker <command>`. A cluster's worker doesn't take backups yet.

## Pairing

An operator issues a pairing code, which works once and for ten minutes. The worker trades it for a key and keeps the key in its state directory:

```sh
LEPTOSKIT_CLOUD_WORKER_PAIRING_CODE=lkp_… leptoskit-cloud-worker pair \
	--url https://cloud.example.com \
	--name builder-eu-1 \
	--capability linux --capability x86_64 \
	--state /var/lib/leptoskit-cloud-worker
```

The pairing code is read from the environment, as above; `--code` works too, but anyone who lists the machine's processes sees an argument. The state directory must be the worker user's own directory, not a link; it is made readable by its owner only, holds one worker, and is locked while a worker runs from it. The control plane's address must be `https`, or `http` on the same machine.

## Running

```sh
leptoskit-cloud-worker run \
	--state /var/lib/leptoskit-cloud-worker \
	--build-command /usr/local/bin/build-leptoskit-server
```

| Option                      | Variable                                 | What it sets                                                                                                                                                                           |
| --------------------------- | ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--state`                   | `LEPTOSKIT_CLOUD_WORKER_STATE`           | Where the key and the job's state are kept.                                                                                                                                            |
| `--workspace`               | `LEPTOSKIT_CLOUD_WORKER_WORKSPACE`       | Where each build's directory is made and deleted. It must be the worker user's own directory, and is made readable by its owner only. `workspace` in the state directory when not set. |
| `--build-command`           | `LEPTOSKIT_CLOUD_WORKER_BUILD_COMMAND`   | The program that builds.                                                                                                                                                               |
| `--build-arg`               |                                          | An argument for it. Repeat for more.                                                                                                                                                   |
| `--build-env`               |                                          | The name of one of the worker's own variables to give it. Repeat for more. Every build sees it.                                                                                        |
| `--build-timeout-minutes`   |                                          | How long one build may run. 60 when not set.                                                                                                                                           |
| `--publish-command`         | `LEPTOSKIT_CLOUD_WORKER_PUBLISH_COMMAND` | The program that publishes to stores. Without it the worker takes no publish job.                                                                                                      |
| `--publish-arg`             |                                          | An argument for it. Repeat for more.                                                                                                                                                   |
| `--publish-env`             |                                          | The name of one of the worker's own variables to give it. Repeat for more. Every publish sees it, and runs the organisation's own code with it.                                        |
| `--publish-timeout-minutes` |                                          | How long one publish may run. 120 when not set.                                                                                                                                        |

The worker stops after the job in hand at the first interrupt or termination signal, and at once at the second.

## What a build is

For each `build_server` job the worker downloads the source archive, checks its size and SHA-256 against the job's, unpacks it into a fresh directory only its user can read, and runs the build command there. The directory is deleted when the build ends. One a worker that was killed left behind is deleted when it next starts.

The command runs with a cleared environment and:

| Variable                 | What it is                                                                                                                                        |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `LEPTOSKIT_BUILD_SOURCE` | The source directory, which is also where the command starts.                                                                                     |
| `LEPTOSKIT_BUILD_INPUT`  | A JSON file: the deployment, the archive's digest, the toolchain, the targets, where to push the image (`image_repository`), and the app's facts. |
| `LEPTOSKIT_BUILD_OUTPUT` | Where the command writes how the build ended.                                                                                                     |
| `HOME`, `TMPDIR`         | Empty directories that are deleted with the source.                                                                                               |
| `PATH`                   | The worker's own, with any variable named by `--build-env`.                                                                                       |

What the command prints, on either stream, is the build's log. It writes one of these to `LEPTOSKIT_BUILD_OUTPUT`:

```json
{
	"result": "succeeded",
	"image": { "repository": "<image_repository>", "digest": "sha256:…" },
	"processes": [
		{
			"name": "app",
			"command": ["/app/server"],
			"port": 8080,
			"ready_path": "/readyz",
			"live_path": "/healthz"
		}
	],
	"migrations": [{ "name": "0001_init", "checksum": "sha256:…" }],
	"steps": [{ "name": "cargo build", "millis": 84000 }]
}
```

```json
{ "result": "failed", "reason": "src/main.rs:4: expected `;`" }
```

A command that exits with a failure and writes nothing fails the build with its exit status. When the command ends, when its deployment is cancelled, and when it runs past its time, the worker stops it and whatever it started in its own process group (`TERM`, then `KILL` ten seconds later). A worker stopped at once by a second signal kills it on its way out. A process that leaves the group, by `setsid` or because a daemon such as Docker's started it, is not stopped: a command that starts one must stop it when it is asked to end.

**The worker doesn't isolate the build.** The command runs as the worker's own user, so it can read the worker's key and reach the network. A process it leaves running can reach the next build's files and output, and every variable named with `--build-env` reaches every build: never name a credential there that one organisation must not use on another's behalf, such as one registry login for every tenant. Lines in a build's log that start `worker:` may be the build's own. To build other people's code, the command has to do the work inside a sandbox of its own, a virtual machine or a container that is thrown away afterwards and is given only the source directory. No such command ships with this crate.

## The build cache

With `--build-cache-dir`, each project's build cache is kept for its next build under `tenants/<organisation>/<project>/` on a volume that every organisation whose builds run on the worker shares. The build command finds its project's directory as `LEPTOSKIT_BUILD_CACHE_DIR`, which holds its `CARGO_HOME` and `SCCACHE_DIR`, and is told the most it may leave there as `LEPTOSKIT_BUILD_CACHE_MAX_MIB`, which the built-in builder keeps to. The worker makes sure that one organisation's cache never decides whether another organisation's build has room:

- A cache export larger than `--build-cache-max-mib` (`LEPTOSKIT_CLOUD_WORKER_BUILD_CACHE_MAX_MIB`) is removed once its build has succeeded. The build's result doesn't change; its next build is only slower. The default is 8192 MiB, a fifth of the 40 GiB volume the platform gives a build worker, which leaves room for a few projects' caches beside it.
- Before each build, the caches of other projects that no build has used for `--build-cache-idle-days` (`LEPTOSKIT_CLOUD_WORKER_BUILD_CACHE_IDLE_DAYS`) are removed. The default is 7 days: a project that still deploys keeps its cache, and one a worker is done with doesn't hold the volume for good.
- Then the least recently used of them are removed until no more than `--build-cache-max-used-percent` (`LEPTOSKIT_CLOUD_WORKER_BUILD_CACHE_MAX_USED_PERCENT`) of the volume is taken. The default is 75, which leaves a quarter of the 40 GiB volume, more than the largest export a build keeps, for the export the build is about to write.

A build never removes the cache of the project it builds: only another project's build can, by the last two rules. The percentage is of the filesystem the cache directory is on, so give the cache a volume of its own: on a disk that is fuller than that for other reasons, every other project's cache is removed before each build. An export is capped once it has been written, not while it is written, so a build whose export is larger than the room that is left fails for want of space, and only that build does.

## Publishing

A build worker bound to an organisation and paired with `--capability publish` (and `--capability macos` on a Mac, for the App Store) is offered that organisation's publish jobs; a shared-pool worker never is. One paired with `publish` refuses to start without `--publish-command`. It is offered publish jobs, and runs each with `--publish-command`, in the unpacked source. The program gets `LEPTOSKIT_PUBLISH_SOURCE`, `LEPTOSKIT_PUBLISH_INPUT` (the job, without the archive's address), `LEPTOSKIT_PUBLISH_STORES` (separated by spaces), `LEPTOSKIT_PUBLISH_SUBMIT` (`1` or `0`), `LEPTOSKIT_PUBLISH_OUTPUT`, a fresh `HOME` and `TMPDIR`, `PATH`, each variable named with `--publish-env`, and the job's store credentials, by the names `lk publish` reads, and `LEPTOSKIT_PUBLISH_CONTENTS_ONLY=1`, so `lk publish` refuses a credential that names a file. It succeeds when it exits with 0 and writes `lk publish --json`'s report to `LEPTOSKIT_PUBLISH_OUTPUT`:

```sh
#!/bin/sh
submit=; [ "$LEPTOSKIT_PUBLISH_SUBMIT" = 1 ] && submit=--submit
exec lk publish $LEPTOSKIT_PUBLISH_STORES --json --yes $submit > "$LEPTOSKIT_PUBLISH_OUTPUT"
```

Each credential's value is hidden in its log and its report. A publish is never run twice, since it may have reached a store: one that a worker dies holding fails. The worker doesn't isolate the program, which runs the organisation's own code with its store credentials: run it inside a sandbox thrown away afterwards, and don't publish on a worker shared between organisations until that sandbox exists.

## A cluster's worker

A worker paired for a cluster applies that cluster's releases and removes its environments. It reaches Kubernetes with the service account of the pod it runs in, or with `--kubeconfig` while developing, and describes the cluster with the platform's defaults (`ClusterProfile::platform_defaults`: Traefik in `kube-system` and KEDA's HTTP add-on in `keda`).

```sh
LEPTOSKIT_CLOUD_WORKER_POSTGRES_URL=postgres://admin:…@postgres.internal/postgres \
LEPTOSKIT_CLOUD_WORKER_POSTGRES_NAMESPACE=postgres \
LEPTOSKIT_CLOUD_WORKER_RUNTIME_CLASS=gvisor \
	leptoskit-cloud-worker run --state /var/lib/leptoskit-cloud-worker
```

| Option                      | Variable                                         | What it sets                                                                                                            |
| --------------------------- | ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------- |
| `--kubeconfig`              | `LEPTOSKIT_CLOUD_WORKER_KUBECONFIG`              | A kubeconfig file to reach the cluster with. The pod's service account when not set.                                    |
| `--postgres-url`            | `LEPTOSKIT_CLOUD_WORKER_POSTGRES_URL`            | The shared Postgres server's administrator. Without it, a release that needs a managed database fails.                  |
| `--postgres-namespace`      | `LEPTOSKIT_CLOUD_WORKER_POSTGRES_NAMESPACE`      | The namespace the Postgres server runs in. Needed with `--postgres-url`.                                                |
| `--postgres-port`           | `LEPTOSKIT_CLOUD_WORKER_POSTGRES_PORT`           | The Postgres server's port. 5432 when not set.                                                                          |
| `--postgres-ca`             | `LEPTOSKIT_CLOUD_WORKER_POSTGRES_CA`             | A PEM file of certificates to trust for the Postgres server's, besides the webpki roots.                                |
| `--runtime-class`           | `LEPTOSKIT_CLOUD_WORKER_RUNTIME_CLASS`           | The `RuntimeClass` every release must run under. Without it a release may keep the cluster's default.                   |
| `--notifications-namespace` | `LEPTOSKIT_CLOUD_WORKER_NOTIFICATIONS_NAMESPACE` | The namespace the cluster's notifications hub runs in. With the next two, the one platform service a release may reach. |
| `--notifications-host`      | `LEPTOSKIT_CLOUD_WORKER_NOTIFICATIONS_HOST`      | The name the hub's Service answers on, as the cluster's `notifications` facts give it.                                  |
| `--notifications-port`      | `LEPTOSKIT_CLOUD_WORKER_NOTIFICATIONS_PORT`      | The port the hub serves on.                                                                                             |
| `--rollout-timeout-minutes` |                                                  | How long a release may take to become ready, a day at most. 10 when not set.                                            |

The worker trusts the control plane only so far. A release whose pods wouldn't run under `--runtime-class`, or that opens its network to any database server but the one `--postgres-namespace` and `--postgres-port` name, is refused before anything is applied. A release bridged to a platform service is refused unless it is the notifications hub the three `--notifications-*` options name, exactly; without them, every release that names one is. A database it is told to make or drop must be the one named for the environment the job names (`env_` and the environment's ID in hex, or `ntf_` for its hosted notifications), so nothing but environments' databases is ever altered; which environment a job is for is the control plane's word.

Before a release that has a managed database, the worker makes sure of its role and database on the shared server: a login role that is nothing more (no superuser, database or role creation, replication or row-security bypass, no settings of its own, a member of no role and with no members, 20 connections at most), whose password is the one leased with the release, sent only as a SCRAM verifier, and a database it owns, made from `template0` closed to connections and opened only once no one else may connect to it. A database of that name that another role owns is never adopted. Every other database on the server that takes connections, `template1` and `postgres` among them, is closed to `PUBLIC`: roles of the operator's that need one must be let in by name. Each of these is checked afterwards, and a step the administrator couldn't make (a managed server where it doesn't own `template1`, say) fails the release. When it sets the password of a hosted notifications tenant's role (`ntf_…`), it ends that role's other sessions, so connections made with an old password don't outlive it: the administrator needs superuser access or the `pg_signal_backend` role. Removing an environment removes its processes from the cluster, then drops its database and role.

An environment whose notifications the cluster's hub serves has a second database and role on the same server, `ntf_` and its ID in hex, made and checked the same way with each release that uses the service, from the password leased as `LEPTOSKIT_NOTIFICATIONS_DATABASE_URL`. That lease is the worker's alone: it is taken out of the values the release's runtime is given, so no process and no Kubernetes Secret holds it, and a release whose lease logs in as another role, or whose password is among the values the processes are given, is refused. Removing the environment drops this database and role too.

`sslmode=disable` in the administrator's address connects without TLS, for a server on the same machine only. Any other mode, or none, requires TLS, with no fallback to plain text, and a certificate that the webpki roots or `--postgres-ca` vouch for.

The shared server has no disk quota per environment, and 20 connections for each environment can use up its `max_connections`. Every environment's role can see the names of the others' databases and roles, and who is connected. Keep the administrator's `pg_hba.conf` entry to the worker's address: every environment's pods can reach the server.

A release whose processes never become ready, or that the cluster refuses, is reported as not ready; the previous release keeps serving. A failed migration shows as the new server never becoming ready: the worker doesn't tell the two apart. Dedicated database sizes (`s`, `m`, `l`) aren't run yet, and a release that asks for one fails. A backup or a restore is answered as failed, which leaves the database as it was.

## Crashes

Before a job runs the worker writes down which job it is, and before it reports an outcome it writes the outcome down. Started again, it sends an outcome it had saved. A build that was interrupted is dropped: its only effect is an image named by its digest, and the control plane gives the build out again when its lease runs out. Any other kind of job is never run twice: the worker refuses to start until an operator has reconciled the job and `leptoskit-cloud-worker forget --state …` has cleared it.
