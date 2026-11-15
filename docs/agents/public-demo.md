# The public demo service

`demo_service` is the public demo's runtime: one process image serves one UTC day's generation, resets itself at midnight by re-executing the same binary in the same PID, and shares a volume with the previous and next images safely through kernel file locks. This page is the operator contract: the environment it takes, the rules a release must follow, and what it deliberately does not do yet.

## The environment

| Variable                         | Meaning                                                                                                                                          |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `DEMO_BIND`                      | The private loopback address to bind, e.g. `127.0.0.1:8080` (bracketed IPv6 accepted). The edge terminates the public origins and forwards here. |
| `LEPTOSKIT_SERVICE_VOLUME_STATE` | The state root: a real directory, mode `0700`, owned by the service account, not a symlink. Refused otherwise.                                   |
| `DEMO_SITE_ORIGIN`               | The site's public origin. HTTPS, except loopback/localhost HTTP for development.                                                                 |
| `DEMO_ADMIN_ORIGIN`              | The admin's public origin, same rules, a distinct authority from the site's.                                                                     |

A misconfigured service refuses to start (nonzero exit) instead of serving half-configured.

The two origins, the bind, and every incoming `Host` header share one host normalization: lowercase, exactly one DNS trailing dot folded away, IPv4 plain, IPv6 canonically bracketed (`[::1]`, whatever the header wrote), and explicit or implicit default ports equal — `https://Notes.Example.:443` and `https://notes.example` are the same authority. Only HTTPS is accepted, or HTTP when the host is a loopback address or a `localhost` development name; no other scheme is accepted anywhere. After normalization the site, the admin, and the bind must still be three distinct authorities — configuration that folds two of them into one is refused.

## What the process serves

One listener, three authorities, validated the same way on every request:

- the **site origin** — minimal pages, sign-in, the identity provider, and `/demo/generation`;
- the **admin origin** — the deploy console's admin, and `/demo/generation`;
- the **bind authority** — only `/healthz`, `/readyz`, and the token-authenticated analytics service prefix. No pages, no admin.

Every other `Host` is refused with 421, before a generation exists and after — including a request whose `Host` header is missing, duplicated, or malformed: exactly one usable `Host` header names the authority, and it is never resolved by silently taking one of several values. `/healthz` and `/readyz` answer on all three authorities. Before a generation is ready, ordinary pages answer 503 with `Retry-After: 5` and `/readyz` names the failing `generation` check.

## The daily generation

State lives under `gen/<UTC day>.f<format>` — one directory per day and format, always at its final path. A generation becomes visible only when its `published` marker is renamed into place, which happens after credentials, databases, the seeded month, and the identity provider's client all exist **and a durability barrier has fsynced every one of them — the databases, their WAL sidecars, and the directory entries up to the volume root** (the SQLite connections run WAL with `synchronous=NORMAL`, where a committed transaction alone is not a power-loss barrier; a barrier failure publishes nothing). Other processes open a generation only after validating that marker against their own format and schema fingerprint — **before** any database is opened, so a mismatch is refused while the data is untouched.

The marker's `seed_events` is a **historical initialization receipt** — proof the build finished and its seed read back — never an invariant over today's analytics. Visitors add events and legitimate deletions remove them; a warm open re-reads the catalogue only to prove the generation answers, so a mutable day restarts and overlaps freely. The initial seed itself must be _complete_: every batch's typed capture answer is checked (nothing refused, nothing rate-limited, every entry accounted), and the read-back must match it exactly before publication.

A marker that exists but cannot be read (corrupt, truncated), or that does not name its own directory, is a **fail-closed fault**: the process stays unready and reports it without touching a database, credential, or marker byte. Only a genuinely absent marker permits the unpublished build path. Collection holds the same rule: an unreadable marker is never a deletion reason.

Three locks, all `flock` files under the volume, all held by the process image rather than by tasks, so neither an aborted task nor the runtime's shutdown timeout can drop one while a writer runs:

- **`build.lock`** — one builder or collector at a time, taken by cancellable polling. A failed initializer keeps it until exec/exit: its background writers may still run, so no other process touches the volume until the image is gone.
- **`<generation>/lifetime.lock`** — held shared by every process with the generation open, from before the first database opens until exec or exit. Collection deletes a generation only if it can take the lock exclusively without blocking, which is how clock jumps, paused candidates, and older formats refuse to have live databases unlinked.
- **`<generation>/worker.lock`** — held by the one process running the control service's worker. A ready candidate polls for it once a second; taking it interrupts operations the previous holder left running.

Retention recognizes a published generation **structurally** — a marker that names its directory with seed evidence — regardless of the format or fingerprint this binary serves. Today and yesterday are kept per format, so a recent generation of a foreign format (the rollback target after a same-day format bump) survives collection with no live holder; only genuinely expired published generations and genuinely unpublished leftovers are deleted, and only under the exclusive lifetime lock. What survives counts against a finite budget of four generation directories and 1 GiB total. When live holders or unreadable markers prevent collection and the budget would be exceeded, the process stays **unready with an honest error** — it never unlinks a live generation and never grows without bound.

## Reset and stopping

The day watch reads the UTC wall clock at most every 30 seconds. When the date differs from the generation's, or a failed initialization's bounded retry (60 seconds) comes due, the image drains in-flight requests and SSE streams for at most 3 seconds, gives the runtime at most 2 more seconds, and **execs the same binary with the same arguments in the same PID**. Every descriptor closes at exec, so the kernel releases the locks and databases only once the old execution has fully stopped. A SIGTERM follows the same drain and exits 0 within 8 seconds.

After a failed initialization the process never retries inside the same image — background tasks may already be running — it keeps the fallback for 60 seconds, then re-executes for a bounded retry. `Restart=on-failure` plus a successful exit is not a reset: the process never exits 0 on its own.

## Release rules

- **Don't release between 23:58 and 00:05 UTC.** A release that observes its candidate during the midnight re-exec sees it change and reverts; retry afterwards.
- **Schema changes need a `GENERATION_FORMAT` bump.** The published marker carries a fingerprint of the build's schemas: the analytics migration list, the deploy console's migration list (`Store::schema_fingerprint()`, computed from the migrations themselves — no duplicate list to keep in step), and the linked SQLite table schema. A binary that finds a same-format marker with a different fingerprint refuses to open the generation, before anything is migrated — the fix is a format bump and a fresh directory, never a silent migration. Adding a deploy-console migration moves the fingerprint, so such a release needs the bump too.
- **Keep the volume's mode and ownership right.** The service refuses a symlinked, foreign-owned, or non-0700 root.

## Deliberately not here yet

- **No visitor onboarding.** The mailer is a sink, there is no development sign-in, and the admin's identity-provider client offers `openid`, `email`, and `profile` only — no `roles` scope, because no one can sign in yet. The pages are plain responses; visual design and personas are later tasks.
- **No cross-process live broadcasts.** Operation progress SSE is process-local. During an overlap, an operation that finishes in the old process is seen by a candidate's viewer through reconnection: the runtime ends successful authenticated operation streams after 15 seconds, so the browser's EventSource reconnects into the durable terminal snapshot the shared store holds.
- **No cloud provider, no real executor.** Operations run against the simulated executor only; nothing contacts a provider.

## Measured, on macOS (arm64, release build)

- Cold start (fresh volume, full seed) to `/readyz` 200: **2.82s** — **over the 2-second graph budget**. Breakdown: ~1.0s process boot (dynamic loading the 49.7 MB unstripped binary and the runtime), and 1.78s of seed: the month goes through the analytics service's own ingestion with committed-ack per batch, and the ingest writer waits up to 50 ms per batch for more rows — 30 sequential batches ≈ 1.5 s of writer wait alone. That path lives in `admin_demo::analytics`, which this slice does not modify; the choices (a smaller seed or a pre-built generation) belong to the parent task.
- Warm start (reopen, no reseed): 0.14s.
- Binary size: 49,721,424 bytes (`cargo build --release`, unstripped; the `dist` profile strips).
- Generation size after a fresh build: ~15 MB.
- Reset outage: measured in debug builds only — the release binary ignores the debug clock hooks, so real midnight cannot be accelerated there without faking a measurement. In debug, the stream closes within the 3 s drain and the new day is ready a few seconds later.
- No Linux or systemd run has happened. The exec/CLOEXEC behavior is argued from the source and asserted by the fd guard and the process tests' Linux-only portions; it is not observed on Linux here.
