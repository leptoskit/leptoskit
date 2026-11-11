# Executable process adapter: integrated local path

The CLI consumes executable contracts through the journaled instance lifecycle described below and supports graph release plan/apply/status/rollback. Checked CLI capture and packaging are integrated and compiled. The [release lifecycle record](graph-release-lifecycle.md) lists the focused Rust/Python validation and the remaining real-host acceptance. This does not close the broader A02-WEB or A02 requirements.

## Implemented behavior

`release/process.py` implements exact installed executable/configuration checks, explicitly declared address environments, private loopback port collision checks, dedicated nonroot systemd service configuration, start without restart, boot enablement after readiness, and exact release-instance stop/disable. HTTP health and readiness observations bind to process executable device/inode, account, process start time and exact loopback socket inode before and after probing. Rootless runtime declarations fail explicitly rather than disappearing from process configuration.

Parent review removed a hardcoded two-name address-environment restriction: the actual playground declares `PLAY_BIND`. The adapter validates and preserves an explicit environment-variable name with the exact loopback address; it does not infer that a binary honors the variable. A new unit test covers `PLAY_BIND` and mismatched values. CLI capture supplies this explicit declaration through its checked executable contract. Real executable host acceptance remains pending.

The Rust transport concatenates the process adapter and existing host worker, and pins the combined source in the plan. No remote module installation or mutable import path is required. The focused Rust CLI/receipt/release suites compile this transport. The normal CLI Python discovery wrapper includes the new tests without a new test command.

The tests start a benign Python HTTP child that consumes `LEPTOS_SITE_ADDR`, with synthetic proc files and a deterministic systemd command adapter. They exercise observations, substituted executable/socket, wildcard listener rejection, restart during probing, port collision before service configuration writes, changed installed bytes, existing unit/drop-in conflicts, failed readiness preventing boot enablement, and exact cleanup identity. They do not start systemd, run an ELF executable, reboot a host or establish real Linux acceptance. The combined-worker load test checks module shipping in Python; focused Rust tests exercise CLI contracts and packaging. Neither establishes real remote-host behavior.

The adapter initially passed 34 Python tests: 21 inherited and 13 new. Parent listen-variable review added a passing regression, for 35 tests before lifecycle integration. All four inherited R30 negative controls were repeated in separate evidence copies: broken SSH field lookup, broken command quoting, retained failed-candidate reservation and omitted restoration probe each failed its targeted regression test. Documentation-template and release-metadata checks passed. The first normal commit hook formatted one Rust function and passed secret scanning; Git correctly withheld the commit because formatting changed the file. The final normal hook and signed commit subsequently passed. Parent integration verified both the adapter and listen-variable fix signatures.

## Required continuation

Capture and the typed `Checked` handle now bind service source association, locks and resolved metadata before enabling service release plans. Preserve independent source reproduction, approvals, API compatibility and attestation states. Private references, schema/identity/migration ownership, database, secret, bind and volume consumption remain fail-closed gaps.

The host lifecycle now implements durable operation/per-unit journals, immutable executable installation, candidate/active route bindings, selected process observation, retirement and rollback. See `graph-executable-release-lifecycle.md` for coverage and limits. Fresh observations of readiness-only dependencies, automatic reboot recovery, and real independent-host acceptance are still required.

Implement exact rootless engine/socket ownership and pinned-image consumption with explicit environment delivery, and prevent compilation children from inheriting runtime sockets. Runtime installation remains an explicit prerequisite, with generated reproducible provisioning still open. Do not enable rootful engine access or claim an image digest proves the host engine.

Add the requested opt-in two-service Linux/SSH release/isolation/rollback fixture. The inherited static SSH fixture remains authored but unrun. Focused CLI compilation/tests have passed; full Clippy, help snapshots, normal repository gates and real Linux/SSH/disconnect/reboot acceptance remain pending on the final revision. This adapter slice did not change schema types; the integrated graph model has regenerated schema aliases, as recorded in the release lifecycle validation.

All original A01–A10, X01–X06, R01–R30 and A02-WEB acceptance requirements remain governed by the canonical audit ledger. This adapter addresses only part of executable runtime mechanics; it is not an audit-remediation completion report.
