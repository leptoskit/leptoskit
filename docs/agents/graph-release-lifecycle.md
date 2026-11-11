# Graph release lifecycle: local validation and deployment acceptance

The CLI consumes the static, SSR and service state machines through `infra release plan/apply/status/rollback`. This record describes the current integrated implementation; earlier handoff notes about uncompiled source and restrictions on running builds are superseded by the validation below and the repository's current `AGENTS.md`.

## Implemented path

Plans are reconstructed from `verify::graph::Checked`, selected artifact inventories, current configuration/placement, typed provider output, reproduced edge files, independently pinned SSH trust and supplied Caddy bytes. Saved JSON is compared with that reconstruction. Strict policy and backend-only trust fail rather than authorizing graph fields. Descriptor staging and ELF inspection do not execute an untrusted backend on the developer's host.

SSH uses explicit known-hosts, a private identity, no user configuration or agent forwarding, BatchMode, StrictHostKeyChecking and a 180-second deadline. A bounded payload invokes the production Python state machine. Root-owned per-unit releases are installed only after inventory checks. Candidate and active routes are probed, and current pointers switch atomically. Caddy uses an existing dedicated UID and an exact generated systemd unit; drop-ins are rejected. Certificates and tunnels remain explicit prerequisites.

Per-host flock and durable journals coordinate prepared and interrupted operations. Recovery rechecks previous inventories, runtime bytes and process ownership before restoring pointers. Full recovery inventories remain on-host; prepare, activate and status return compact journal summaries so an otherwise permitted asset tree cannot overflow the 1 MiB SSH response limit. Failed SSH records no successful local history. Hosts commit independently, and shared edge configuration changes are rejected during selective releases.

SSR/service candidates have immutable per-release runtime bindings, distinct candidate addresses, readiness checks and recoverable promotion/retirement. The CLI includes their payloads and recovery paths. [Executable lifecycle details](graph-executable-release-lifecycle.md) describe the process boundary. A readiness observation does not establish source provenance or runtime attestation.

## Validation

Before the response-size correction, 156 configuration/schema tests, 99 focused Rust CLI/receipt/release tests, and 89 Python release/recovery tests passed. These Rust suites compiled the integrated CLI library and its Python wrappers. The generated schema aliases were regenerated and checked. This supersedes the old status that the CLI wrapper, ELF tests and executable integration were uncompiled.

The added transport regression executes the production host worker through the real bounded `exchange` function, deploys two 10,001-file trees, retries preparation and activation, and reads status while retaining the complete prior inventory on-host. Existing transport tests still exercise exact remote command construction, pinned SSH arguments, simultaneous large upload/reply, limits, timeouts, remote failures and incomplete responses. The runtime adapter is deterministic; these tests do not establish real SSH, systemd, Caddy, TLS or cloud behavior.

Existing state-machine tests cover install/retry/rollback, failed candidate and active readiness, copied-byte substitution, altered installed artifacts, restrictive umasks, operation conflicts/flock, selective route isolation, changed runtime/edge/host bindings, failed restoration and interruptions across promotion. Negative controls for earlier SSH lookup/command, journal reservation and restoration-probe bugs remain recorded in the historical acceptance evidence. Reboot and real connection loss need separate live evidence.

## Remaining acceptance and resources

Run the full CLI snapshot and repository gates on the final revision. `crates/leptoskit_cli/src/infra/graph/release/acceptance.py` is an opt-in real Linux/SSH fixture for two releases, status, public HTTPS bytes and rollback. It remains authored rather than accepted against a real deployment. Provider fixtures do not establish live DNS, TLS, firewall or production cutover.

Graph receipts establish selected artifact integrity and binding. Rootless image execution, source reproduction, origin/publisher authorization, independent graph approvals, cross-app API compatibility and runtime attestation remain outside this path. Graph services still require `database = false`; broader A02/A04/A05/A09 acceptance remains open.

Use the current repository build instructions with `CARGO_BUILD_JOBS=4` and mbx-managed targets. Keep expensive builds coordinated and preserve source/evidence outside disposable build caches. Signing and hooks remain enabled. Static transfers are bounded to 64 MiB per host including Caddy; certificate/tunnel setup, brief edge restart and separate shared-edge migration remain operational constraints. Failed staging and old releases remain for explicit operator cleanup. Root owns the installation trust boundary, and rollback rechecks original artifacts against current configuration and trust.
