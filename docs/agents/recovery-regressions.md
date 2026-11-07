# Recovery regression acceptance

R17–R19 were integrated on the services-enabled main branch after RPC release compatibility (#608). These fixes do not establish online expand/migrate/contract migrations, primary failover, or cross-host availability. A05 and the broader A09 acceptance remain open.

## Local acceptance

On 7 November 2026, the following commands passed with `LEPTOSKIT_REQUIRE_POSTGRES=1` and `LEPTOSKIT_REQUIRE_RESTIC=1`, with stale `PGBIN` and `LEPTOSKIT_POSTGRES_BIN` removed. Commands ran inside `devenv shell`, using two build jobs and four test threads.

- `cargo test -p leptoskit_cli --lib infra:: -- --test-threads=4`: 63 passed, including generated-unit tests, the seven Python status cases, encrypted Postgres/Restic recovery, and services database isolation.
- `cargo test -p leptoskit_cli --test infra -- --test-threads=4`: 49 passed, including verification failure before repository initialization or timer enablement, backup/restore commands, and service deployment lifecycle tests.
- `cargo test -p leptoskit_db --features sqlite,postgres --test migrations -- --test-threads=4`: 15 passed. Both drivers refused consolidated rollback without changing data or history, then migrated successfully. Ordinary rollback, transactional reversal, and checksum coverage remained green.

The encrypted recovery test uses real Postgres and Restic with the local engine fixture. It is not a cloud deployment or a Docker production recovery exercise.

## Linux systemd acceptance

A task-owned Podman Linux VM (two CPUs, 3 GiB memory, systemd 259) received the exact old CLI-generated service and timer and fresh output from the fixed CLI's `infra backups install` command using fake infrastructure transports.

`systemd-analyze verify --man=no` rejected the old artifacts for a missing `ExecStart` and malformed install section, and accepted the fixed artifacts. `systemctl --root=<fixture> enable` and `is-enabled` confirmed timer enablement and the `timers.target.wants` symlink. The VM's real manager reported `Triggers=leptoskit-notes-production-backup.service`. The timer remained inactive; this validates parsing, installation and linkage, not scheduled backup execution. The task-owned VM was stopped and removed afterward.

SHA-256 artifact digests:

| Artifact      | Digest                                                             |
| ------------- | ------------------------------------------------------------------ |
| Old service   | `e81f34ade78a57e0f2f48783e538289eb652debd081587b8cd6172b7e078cf64` |
| Old timer     | `8b040f30e3090a0853dada45150af25b52c1571c586570797f293a56d7a3cafe` |
| Fixed service | `87d0ac443435ce181f5c5e9926a7928295ef96a6cd280ddfc433137b9c094733` |
| Fixed timer   | `f8213c1a9f0329020e3fdb6ecd1f8758581ed98c525bf8a7b2d5749ee444be06` |

## Revalidation after evidence loss

The earlier local logs were externally deleted and are absent evidence. On 7 November 2026, the four patches were rebased onto main `7e8a7c7b`; range-diff reported all four unchanged and all four signatures verified. The revalidated implementation is `6a034f2b`.

Fresh focused results: 63 CLI unit tests, 49 command tests, and 15 SQLite/Postgres migration tests passed, each with zero ignored tests. Postgres and Restic were required. The real encrypted restore test passed. Fresh systemd 259 acceptance rejected the exact old CLI artifacts, accepted freshly generated fixed artifacts, enabled the fixture timer and confirmed real-manager linkage with the timer inactive. Its artifact hashes match the table above.

Reproduction scripts and non-secret unit fixtures are committed in [recovery-acceptance](recovery-acceptance/). Run `focused.sh` inside devenv with an exclusive target and the pinned nightly toolchain. `generate.py` invokes the actual CLI using transports extracted from the committed command tests. `systemd.sh` runs only inside a disposable Linux VM as root. The VM used two CPUs and 3 GiB; it does not exercise scheduled backup execution or production Docker recovery.

New logs live outside generic caches at `/Users/mini02/Developer/recovery-revalidation-20261107/logs`, with an ownership note and checksum index in the parent directory. This note does not enforce global protection. Mini03 SSH backup was unavailable: its existing proxy reported no Tailscale IPv6 address for the host. Small acceptance records and fixtures are therefore retained in signed source commits as the fallback. No lost log has been reconstructed or counted as fresh evidence.

Fresh focused log SHA-256 values:

- `cli-units.log`: `5b04db083d92f92d41afec77cab42c661a7896162c5d1ad82d98c127eb3794b1`
- `infra.log`: `f9086e450fe4c6ba4ded18c2cc37c6cc36b428d8964c3b4d0d13e614bf03b7e2`
- `migrations.log`: `442f4d0d59da511d74b3cb57293a7ad32955fc4fee6f23e415b23b5d76baa10a`
- `systemd-2.log`: `fbd40f3f9c66d96ea7aadc478fba689cfed3a996a68ba984c3bc5328a5f32fd7`
