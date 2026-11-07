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

The implementation checkout retains commands, environment, CLI binary digest, artifacts, negative controls, and logs under `.git/recovery-evidence/`: `targeted.sh`, `targeted.log`, `generate-fixed.py`, `fixed-cli-result.json`, `run-systemd.sh`, `systemd-acceptance.sh`, `systemd.log`, and `vm-cleanup.log`. These local files are not required to build the repository. The original parent probes remain separately preserved there; they are not database runtime proof.
