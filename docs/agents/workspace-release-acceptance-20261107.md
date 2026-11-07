# Workspace release acceptance, 7 November 2026

Acceptance is incomplete. No PR was published and no cloud resources were applied.

The exclusive checkout is `audit-workspace-releases`, branch `feat/workspace-independent-releases`. The source bundle required public commit `f980cf794a20c534a9f6b0be401401640e06c815` and passed `git bundle verify`. The three source patches were rebased with `git rebase --gpg-sign origin/main` onto `8051706d`. `git range-diff` reported all three patches unchanged:

- `9e6b57ed` became `013f32b08d00d0696aad714c793dba0fa7c36e54`.
- `724a7ed9` became `cbb0682d73d71695b7260ea78255d8bb22d57611`.
- `2d6bf1b2` became `19bc6d25491f813ede0d8a785b2849c11533581a`.

All three rebased commits reported good signatures. The original source bundle and an additional integrated bundle retain the source and integrated revisions. The merged recovery/systemd transport implementation was not modified.

## Gate attempts

Raw evidence is in `/Users/mini02/Developer/workspace-release-acceptance-20261107`, outside caches. `env.sh` records the requested toolchain and resource limits. Commands were run through `devenv shell` after `devenv shell -- true`.

The first `devenv shell lint:push` passed secret scanning and found two existing macOS compilation issues: the menu test compared a result whose success type lacks `Debug` and `PartialEq`, and Linux shortcut helpers were unused on macOS. The pending fixes use a pattern assertion and compile the helpers only on Linux or in tests. No checks were disabled. A second normal gate waited for the first run's remaining Clippy jobs.

Both gate sessions subsequently exited 255. The normal pre-commit formatter was killed with signal 9, so the fixture/fix commit did not complete. The supplied read-only toolchain directory disappeared during these runs: `/Users/mini02/Developer/projects/leptoskit/audit-recovery-regressions/.git/recovery-evidence/rustup`. No task command deleted it. The parent must restore the exact backed-up toolchain before validation and signed commits can continue. Neither gate passed; the actual pre-push hook has not run.

## Real engine preparation

The task created only `workspace-release-20261107` using:

```sh
podman machine init --cpus 2 --memory 3072 --disk-size 12 workspace-release-20261107
podman machine start workspace-release-20261107
podman pull docker.io/library/alpine:3.21
podman pull docker.io/library/rust:1.85-alpine
```

The Alpine image ID was `sha256:ed2dcc5679354d1c17b64f71343e311652c575a6551bbce979eab00644a14eee`; the Rust builder image ID was `sha256:4b3c705bafc50e801b1c69e6a17a3ac51bec2391ec1201498e963cb929f3ffd0`. Kamal 2.12.0 was installed into the evidence directory with `SDKROOT` set from `xcrun --show-sdk-path`; the initial gem installation without that SDK failed. Docker 28.5.1 ran under the task-owned systemd unit in the VM, and a real local registry container started. SSH transport to the VM was exercised. The VM then stopped unexpectedly alongside the gate interruptions. Its shell's Docker/Podman command selection still needed correction.

No verified fixture image was packaged or deployed during this attempt. No two-app, database-history, unchanged-container, negative-control, or Kamal acceptance is claimed. The new serving fixture is preparation only. Full A02 still requires automatic builds, change impact, private bindings, caller identity, physical schema ownership and cross-app compatibility. Provider provisioning remains a local fixture boundary, not cloud acceptance.

The restart failed because gvproxy's process/socket had disappeared. Cleanup ran `podman machine stop workspace-release-20261107` followed by `podman machine rm --force workspace-release-20261107`; both exited successfully. Only the task-owned VM and its images were removed. Shared targets, toolchains, and evidence were not removed. Pending changes remain staged because the normal commit hook requires the missing toolchain.
