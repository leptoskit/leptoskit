# Graph static release lifecycle — acceptance pending

Base: signed parent `99f947e4f8cf2b29fad83e13d33a7a73758c726c`. All nine saved source files were restored after checking their manifest SHA-256 values. No old worktree or auth source was edited.

## Implemented source

Descriptor-based private staging and non-executing ELF inspection remain. `infra release plan/apply/status/rollback` reconstructs plans from `verify::graph::Checked`, selected static inventories, current configuration/placement, typed provider output, reproduced edge files, independently pinned SSH trust and supplied Caddy bytes. Saved JSON is compared, never accepted as a verification token. Strict policy and backend-only trust fail rather than authorizing graph fields.

SSH uses explicit known-hosts, a private identity, no user configuration/agent forwarding, BatchMode, StrictHostKeyChecking and a 180-second deadline. A bounded payload invokes the production Python host state machine. Private staged copies become root-owned per-unit releases after inventory verification. Candidate and active routes are probed; current pointers switch atomically. Caddy uses an existing dedicated UID and an exact generated systemd unit, with drop-ins rejected. Certificate/tunnel installation remains a prerequisite.

Per-host flock and durable journals coordinate prepared/interrupted operations. Per-operation records and selected per-unit journals retain state. Recovery checks previous inventories/runtime bytes before restoring pointers. Failed SSH records no local successful history. Hosts commit independently. Shared edge configuration changes are rejected to prevent docs-only releases changing unrelated routes.

## Validation and remaining gates

Twenty-one Python tests pass through devenv. They execute the production state machine with a deterministic runtime adapter: install/retry/rollback, candidate/active readiness failure, copied-byte substitution, repeated version with different bytes, installed tampering, extra files/modes, prerequisite rejection, operation conflicts/flock, docs-only isolation, changed runtime/edge/host bindings and nine interruption points. Transport tests execute the exact remote shell command locally and cover pinned SSH arguments, simultaneous large upload/reply, output limits, timeout, remote failure and incomplete responses. They do not establish real SSH, systemd, Caddy or TLS behavior. A CLI unit test invokes these Python tests so they cannot be omitted from the normal Unix CLI suite; that Rust wrapper remains uncompiled.

Parent review fixed malformed SSH command construction and trust-field lookups. The transport now bounds output during transfer and rejects incomplete activation responses. A failed candidate releases its journal reservation only when all current pointers still match the previous deployment, permitting a corrected release or rollback. Failed restoration keeps a nonterminal journal until the previous routes pass readiness. The generated service is enabled for reboot; removing the first failed activation disables it. Explicit directory modes also preserve public asset traversal under a restrictive installer umask. Four negative controls restore the broken SSH lookup, remote command, failed-candidate reservation and missing restoration probe; each fails its regression test. Tests cover these transitions, but real reboot and SSH evidence remain required.

`crates/leptoskit_cli/src/infra/graph/release/acceptance.py` is an opt-in real Linux/SSH fixture. Its docstring specifies private inputs. It checks two releases, status, public HTTPS bytes and rollback. It is authored, not run. Restored Rust ELF/staging tests and CLI integration remain uncompiled. CLI help snapshots must be generated and reviewed after a build grant; they are not hand-edited. Configuration schema types are unchanged.

Required gates: CLI compilation/tests/Clippy; actual help snapshot generation/review; full lint/test/docs gates; Linux/SSH acceptance for HTTPS and tunnel; Rust-boundary stale configuration/provider/host-key/receipt negatives; real connection loss across promotion. Python binding tests do not replace these checks.

A02-WEB and parent A01–A10/X01–X06/R01–R29 remain open. The host worker now has SSR/service instance mechanics documented in `graph-executable-release-lifecycle.md`, but the CLI does not yet consume them. Rootless engines/image binding, live readiness-only observations, source reproduction, origin/publisher authorization, graph approvals, API compatibility and runtime attestation remain unimplemented. Local route probes are observations, not attestation or public network acceptance.

Static limits: 64 MiB per host including Caddy; existing account/certificates/tunnel; candidate port 18088; brief edge restart; separate migration for shared edge configuration changes. Failed staging and old releases remain for operator cleanup. Root owns the installation trust boundary; releases are immutable by convention. Rollback rechecks original artifacts against current configuration and trust.

## Resource contract

No workspace/CLI cargo build/check/test/clippy, container or VM is authorized. Only the normal pre-commit formatter may reuse the supplied shared target with jobs=1. Signing/hooks remain unmodified. Final handoff records source/evidence backups outside broad Caches and the signed commit outcome.
