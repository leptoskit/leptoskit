# Security review of Control 2 (uploads, deployments, the worker protocol)

An independent reviewer traced these through the source of the control plane's second slice. Paths are under `crates/leptoskit_cloud/src/`; line numbers are from the reviewed commit and will have shifted.

## Findings

### 1. High: outstanding upload tickets bypass the organisation's storage cap

`server/uploads.rs` around line 194.

A deploy-capable caller: (1) calls `begin_upload(digest=D, size=N)` for a large archive and keeps the ticket; (2) calls `begin_upload(digest=D, size=1)`, which replaces the reservation with one byte without invalidating the first ticket; (3) uploads the valid N-byte archive with the first ticket; (4) repeats for different digests. The store holds N bytes per archive while the cap counts one. No race or successful completion is required.

There is also a completion race: `complete_upload` captures size N, verifies outside its transaction, then marks the current row verified without checking that its size or identity is unchanged (around lines 239 and 351). A concurrent second `begin_upload` can leave a verified N-byte archive charged as one byte.

Reviewer's smallest fix: make an existing digest's size immutable and reject a conflicting size; carry the archive row ID and expected size through verification and require both to match before committing.

### 2. High: one tenant can block unrelated clusters' job queues

`server/workers.rs` around line 315.

`claim` selects the oldest 100 queued jobs by role, then applies pool, cluster and capability matching in memory. A tenant can create 100 deployments, have their builds finish, and leave their release jobs queued on cluster A, whose worker is unavailable. A later release on cluster B falls outside that first page, so B's healthy worker repeatedly receives nothing. Queued releases have no expiry, so the blockage persists. The project and environment caps permit this. Organisation-bound build workers suffer the same behind other organisations' queued builds.

Reviewer's smallest fix: apply role, organisation and cluster eligibility before `LIMIT`; paginate past capability-ineligible jobs rather than treating the first page as the whole queue.

### 3. High: log storage has no job or organisation bound

`server/workers.rs` around line 518.

A compromised build worker claims a job, renews it periodically, and sends successive valid `append_log` calls with increasing `seq`, each up to 64 KiB. Every chunk is another row and object. There is no cumulative byte or chunk cap, and renewal has no maximum job lifetime. The 5 GiB storage cap counts only archives. Thirty-day retention does not bound the ingestion rate or an active job.

Reviewer's smallest fix: enforce cumulative per-job and per-organisation log budgets transactionally before reserving a chunk; give build attempts a maximum execution deadline that renewal cannot extend.

### 4. Medium: build outcomes can select any image repository

`release.rs` around line 477.

A build worker claims a job whose `image_repository` is the server-assigned organisation/project repository, then calls `finish` with an output naming another repository, in `output.image` or a per-process image override. `built` passes it to `release::spec`; runtime validation checks only that the repository is non-empty and the digest is well-formed. The cluster assignment carries the substituted repository and the Kubernetes adapter uses it directly.

Consequence: a build result escapes its assigned image namespace and can make the cluster pull arbitrary registry images (access to another tenant's private image further depends on registry pull permissions).

Reviewer's smallest fix: compare every selected process image repository against the job's assigned repository before accepting the outcome.

### 5. Medium: expired build leases still authorise renewal, logs and completion

`server/workers.rs` around line 376.

A build worker claims attempt 1, waits until `lease_expires_at` passes, then calls `renew`, `append_log` or `finish` before the sweeper processes the row. `held` checks ownership and `state == Leased` but never the expiry; `renew` revives the expired lease; `finish` (around line 616) accepts `Leased` without checking expiry, so a late build finish can still queue a release. Authority expires when the sweeper happens to run rather than at the lease deadline.

Reviewer's smallest fix: enforce `lease_expires_at > now` in the transactional ownership check and the build completion path; keep the deliberate late-completion exception only for unknown cluster jobs.

### 6. Medium: rollback ignores explicitly uncertain migration state

`server/deployments.rs` around line 961.

Deployments #1 and #2 went live with migration history `[A]`; #1 is superseded. #3 carries `[A,B]`, applies B during startup, then fails readiness. The cluster reports not-ready; the control plane keeps `[A]` and sets `schema_uncertain = true`. A developer calls `rollback(to=1)`. `rollback_target` compares the retained `[A]` with #1's `[A]` and accepts, never checking `schema_uncertain`; the rollback assignment declares that no migration work is needed.

Consequence: a rollback presented as schema-compatible is accepted against a database that may already contain B.

Reviewer's smallest fix: refuse rollback while `schema_uncertain` is set, until reconciliation establishes the applied history.

### 7. Medium: a valid runtime-log report produces an unreadable object

`server/workers.rs` around line 663.

A cluster worker calls `report_status` with 500 lines of 4 KiB each, within the line-count and line-length limits. The handler stores about 2 MiB as one object. Both store readers reject objects over `MAX_SMALL_OBJECT` (1 MiB), and `runtime_logs` propagates that error when a page meets the chunk.

Reviewer's smallest fix: split reports into objects under the limit, or reject reports whose serialized size exceeds it before creating index rows.

### 8. Medium: failed teardown is permanently treated as completed

`server/workers.rs` around lines 626 and 638.

An authorised person deletes an environment. The cluster worker claims its `destroy_environment` job and finishes with `DestroyOutcome::Failed { reason }`. `finish` marks the job `Done`; the destruction outcome branch does nothing. The environment and placement rows are already deleted. The default operator job listing excludes `Done`, the sweeper ignores it, and `reconcile` rejects it because it is not `Unknown`.

Consequence: a failed deletion leaves workloads, routes and possibly persistent data running with no remaining environment or recovery operation.

Reviewer's smallest fix: keep a failed teardown as an actionable, retryable job and expose its failure; only `Destroyed` completes teardown.

### 9. Medium: archive sweeping can delete an archive whose retention was renewed

`server/jobs.rs` around line 504.

The sweeper reads an expired archive into its candidate list. Before its per-archive transaction starts, a caller uses `begin_upload` on that verified digest, extending retention and receiving `Stored`. The sweeper re-reads by ID but never re-checks `expires_at`; with no in-flight deployment, it deletes the renewed archive. Separately, object deletion happens after the row deletion commits: a fresh upload of the same digest can recreate and verify the same object key in that gap, and the older sweep then deletes its bytes.

Reviewer's smallest fix: re-check expiry transactionally, and coordinate object deletion with a tombstone or a generation-specific object key so an older deletion cannot remove a replacement.

## Test and coverage gaps the reviewer named

- The authorisation matrix exercises persons, CLI callers, scoped and project-bound tokens, anonymous callers and one build-worker identity; it does not run the full matrix with cluster-worker identities, and its worker checks mostly use non-existent job IDs.
- No test establishes: outstanding-ticket reservation safety, completion-versus-reservation races, lease expiry enforced before sweeping, queue progress past 100 ineligible jobs, aggregate log budgets, oversized status chunks, failed-teardown recovery, or refusal of rollback under uncertain schema.
- Log redaction works per line with no cross-line or cross-chunk state, so a split credential can evade it. Log text is returned without terminal-control sanitisation.
- Migration fingerprints are worker assertions, not attestations tied to the returned image or the database's actual history.

## Checked and found sound

Tenant lookup order (the database-resolved organisation and project are authorised before resources are fetched; deployment numbers and cursors stay within the authorised project and environment; `in_flight` filters project-bound tokens). Credential separation (worker identity is distinct; organisation and operator operations reject it; worker operations reject people and tokens; claim eligibility checks role, pool and cluster). Pairing (128 random bits, hashed, ten-minute expiry, role/pool/cluster inherited from the stored pairing, redemption and worker insertion in one serializable transaction). Receipts (`pair`, `issue_pairing`, `begin_upload` and `claim` are actions; their credential-bearing results are not persisted). Upload integrity and signatures (organisation-specific keys prevent cross-organisation deduplication; local PUT streams through SHA-256; S3 verification uses a checksum or bounded streaming; the local HMAC binds method, bucket, key, size, digest and expiry with constant-time comparison; key validation excludes traversal; S3 PUT signs content length and checksum). Lifecycle structure (transitions go through `Changing::advance` with events in the same transaction; both migrations enforce unique in-flight and live slots per environment; unknown release jobs are not replayed; reconciliation prevents a later worker finish from replacing its decision). Release authority (environment identity, namespace, hostname, routes, variable names, resources and quota come from server-held context and plans, not the build outcome; commands and arguments reach structured container fields). Binary configuration (production requires storage configuration; directory storage is `--dev` only; dev binding is loopback-only).
