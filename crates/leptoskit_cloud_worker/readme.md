# leptoskit_cloud_worker

`leptoskit-cloud-worker` is LeptosKit Cloud's worker. It pairs with a control plane, asks it for jobs, runs one at a time under a lease, sends the job's log, and reports how it ended.

This crate holds the loop every role shares and the `build` role. The `cluster` role, native builds and store publishing aren't built yet.

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

| Option                    | Variable                               | What it sets                                                                                                                                                                           |
| ------------------------- | -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--state`                 | `LEPTOSKIT_CLOUD_WORKER_STATE`         | Where the key and the job's state are kept.                                                                                                                                            |
| `--workspace`             | `LEPTOSKIT_CLOUD_WORKER_WORKSPACE`     | Where each build's directory is made and deleted. It must be the worker user's own directory, and is made readable by its owner only. `workspace` in the state directory when not set. |
| `--build-command`         | `LEPTOSKIT_CLOUD_WORKER_BUILD_COMMAND` | The program that builds.                                                                                                                                                               |
| `--build-arg`             |                                        | An argument for it. Repeat for more.                                                                                                                                                   |
| `--build-env`             |                                        | The name of one of the worker's own variables to give it. Repeat for more. Every build sees it.                                                                                        |
| `--build-timeout-minutes` |                                        | How long one build may run. 60 when not set.                                                                                                                                           |

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

## Crashes

Before a job runs the worker writes down which job it is, and before it reports an outcome it writes the outcome down. Started again, it sends an outcome it had saved. A build that was interrupted is dropped: its only effect is an image named by its digest, and the control plane gives the build out again when its lease runs out. Any other kind of job is never run twice: the worker refuses to start until an operator has reconciled the job and `leptoskit-cloud-worker forget --state …` has cleared it.
