# Executable graph release state machine

The production SSH worker now installs and activates executable instances through the same durable state machine as static files. This is implementation and deterministic acceptance, not a working end-to-end CLI claim. The CLI integration now consumes checked asset/executable inventories and resolved service evidence. Focused Rust compilation, generated schemas and command/help tests now pass as recorded below; full gates and real host acceptance remain pending. No audit item closes.

## Candidate and active instances

The host receives a checked requested process contract, exact executable bytes and any public assets. It writes the executable, original process contract and effective process contract as distinct members of an immutable release directory. The requested listen environment is explicit, including `PLAY_BIND`. The effective contract replaces only that variable and its matching loopback address. Declared public settings remain literal; the worker quotes values and escapes systemd percent specifiers. Shell expansion, control characters and arbitrary systemd directives remain unsupported.

Each candidate receives a separately allocated loopback port. Allocation is not a reservation: a racing listener makes startup or process/socket-bound readiness fail, leaving the previous release active. The port and effective process configuration are retained with the installed release, so retries use the same instance. This implementation does not automatically allocate a replacement after a collision.

The previous process remains running during candidate startup. Candidate routes use the same generated Caddy template with selected upstream addresses replaced. Unselected upstream addresses are carried from the current template-checked configuration. Changes to route structure, TLS policy, domains, paths or headers are rejected. An executable's public health/readiness endpoints must be inside its declared route mount. Private executables have direct process-bound probes but no public-route probe.

The worker journals derived process contracts and the rendered edge digest before reporting preparation. Activation rechecks those values, starts or observes the candidate, checks candidate routes, switches selected unit and edge pointers, checks active routes and process readiness, and enables the new instance for boot. It records retirement intent before stopping and disabling the old instance. Only then does it record completion. Process observations explicitly retain absent attestation.

## Failure and recovery

Failed candidate preparation stops only candidate instances covered by the operation, leaving previous instances and pointers intact. A failed cleanup retains the nonterminal journal and operation ownership. A first activation interrupted before edge service installation can recover without trying to stop an uninstalled unit. Existing edge/application unit substitutions or drop-ins prevent retirement rather than targeting an unrelated service.

Interrupted promotion or retirement verifies previous file and process pins, restarts and observes the previous process if it was already retired, restores and probes old routes, then stops the rejected candidate. Failed restoration never records successful rollback. Retrying a completed release observes its process again. Rollback starts the original installed executable with its original process configuration; it does not rebuild it.

Independent unit histories remain scoped to the selection. Tests cover a compiler-only update and rollback without starting, stopping or changing the accounts service, and a docs-only update without touching the running compiler or its effective route. Hosts still commit independently. Prepared candidates on an earlier host can remain pending if preparation on a later host fails; the journals describe that partial state. There is no automatic fleet-wide transaction or reboot reconciler yet.

## Evidence and remaining work

The initial integrated cheap suite passed 52 tests; the current worker suite has 89 passing tests. Six negative controls remove candidate cleanup, old-instance retirement, old-instance restart during recovery, preservation of unselected routes, derived-input checks and the uninstalled-edge guard; each makes its targeted regression test fail. Focused Rust acceptance now passes as recorded below; full gates and real Linux acceptance remain pending.

The Python suite executes the real file/journal/routing state machine with deterministic process and edge adapters. It covers immutable payload/configuration/mode rejection, separate candidate ports, coexistence, failed readiness and cleanup, fresh boot enablement, retirement, retry/rollback, old-input substitution during recovery, selected-only isolation, SSR assets with an executable, and twelve interruption points through post-retirement completion. Adapter tests separately use real benign local HTTP responses with synthetic Linux proc/systemd state. These tests do not prove real systemd, Linux ELF, SSH, Caddy routing, TLS, reboot or device behavior.

Required next work remains: complete CLI-to-Linux release acceptance; existing-host account/runtime prerequisites; fresh readiness-only dependency observations; rootless engine/socket and pinned-image delivery; private bindings and service data/identity/migration ownership; graph source reproduction and approvals/API policy; full CLI/workspace coverage and Clippy; normal full gates; and real two-service Linux/SSH failure, rollback, disconnect and reboot acceptance. Runtime attestation remains absent. The public playground is not deployed by this change.

## Independent review corrections

Independent review established two worker defects with failing deterministic fixtures. Reusing a previous plan's edge directory during rollback could restore a stopped, unselected service's old port. Also, a rejected request with the same operation/host but a different version could enter cleanup and cancel the original prepared candidate. The second defect was reproduced at the worker boundary; the existing Rust caller already includes version in its plan-derived operation digest.

A desired plan's operation identity is now separate from each transaction identity. A new attempt or rollback gets a new immutable edge directory, derived from the current unselected route bindings and selected installed executables. Transaction records preserve earlier attempts even when the same plan is used again. Before promotion the worker checks the previous pointers and pinned edge inputs and recomputes the expected preserved bindings. It never edits or reuses an old transaction's route table for a new rollback.

Action/type, version, host binding and in-flight rollback intent are checked before mutation/cleanup handling. Cleanup also requires the failed journal to match that complete request context. Rust activation requests now carry the rollback flag consistently with preparation. Tests cover interleaved two-service releases followed by rollback, retained old edge/history bytes, rejected request isolation, and changes to the previous edge between preparation and activation. Real SSH, systemd and Rust acceptance remain pending.

## Old-worker journal recovery

The follow-up review reproduced an upgrade regression using the actual prior worker: a prepared rollback had neither `rollback` nor `transaction` metadata and was rejected as an ordinary release. The worker now distinguishes that legacy journal from a modern intent mismatch. It validates operation/version/host ownership, installed candidate pins, previous release pins and permitted pointers before changing processes. It records the newly supplied intent, restores the prior state and starts a fresh transaction. A direct activation retry performs this fresh preparation first; it cannot promote the legacy route table. An interrupted recovery retains exclusive ownership and the recorded intent.

The actual previous-worker reproduction now passes. The expanded deterministic suite has 61 passing tests, including direct activation, prior/candidate substitutions, context mismatches, interrupted recovery and stale unselected route preservation. These are local worker tests, not Rust compilation or real Linux acceptance. Executable CLI integration and the broader acceptance requirements above remain open.

## Checked CLI integration pending acceptance

The CLI now derives contracts from `Checked::release_inventory` and `Checked::executable_inventory`, inspects each selected backend for its independently declared host architecture without executing it, and retains the `Checked` and `Staged` handles through packaging and activation. The bounded transport includes exact staged backend bytes and public assets under one 64 MiB per-host budget including Caddy. Services with no assets do not require a fake asset source directory. ELF format and required-library metadata describe the binary; installed libraries and runtime readiness remain host observations.

An executable declares `serve.address-env`, for example `PLAY_BIND` or `LEPTOS_SITE_ADDR`, and must consume that variable as its complete loopback socket address. This field is optional for graph inspection/builds and older configurations, but required for executable release. Omitted fields do not change old serialized configuration. The captured resolved service settings are checked for unsupported inherited database, storage, bindings and secret requirements before a plan is emitted.

Provisioning and releases derive the same stable dedicated account from project and qualified unit identity. Generated cloud-init adds locked system accounts with no login shell or home directory, scoped to each host, and the edge account for public hosts. This describes new host initialization; changing user data on an existing server does not establish that the accounts were created. Existing-host upgrade delivery and actual cloud-init execution remain acceptance work.

Dependency ordering/readiness observers, mounted executable probe paths, rootless runtime delivery, service secrets and persistent data remain explicitly rejected. SSR asset-root configuration and its real serving journey still need acceptance. No placeholder success or runtime proof is emitted for these gaps. Source reproduction, graph approvals and cross-service identity/API/schema compatibility remain separate work.

Authored Rust tests cover declared socket variables, exact backend pins, unsupported inherited settings, account placement and bounded backend-only/static transport. They have not run. The required `schema:update`, Rust tests/Clippy, CLI snapshots, full gates and real Linux/SSH acceptance must run in the allocated build slot before publication. The earlier worker-only evidence is not evidence for this new Rust integration.

## Explicit historical CLI recovery

R36 exposed a separate CLI boundary: regenerating a plan changes the operation hash, while adding even empty executable fields changes the host binding. The worker's legacy recovery alone therefore could not be reached after an upgrade.

`infra release apply`, `rollback` and `status` now accept `--recover-plan-sha256 <independently-retained-sha256>` alongside the original `--plan` and the same bundle, version, selection, trust and Caddy arguments. Keep the original generated infrastructure files until recovery finishes. Do not run `infra generate --force` over those files first. The extra pin identifies the exact original plan bytes; it is not a signature, publisher authorization or source reproduction claim.

Recovery checks the retained generated file membership and hashes instead of regenerating historical provider files with today's generator. Current configuration and typed provider outputs must still match; normal provisioning continues to require current generated bytes. Current checked artifacts, host trust, executable contracts and routing must match the original plan. Only the known static-to-general plan kind, absent empty executable fields and historical worker/transport hashes receive explicit compatibility treatment. Historical hashes retain operation identity; the transport runs the current worker, never code supplied by the plan.

The original operation must still own the host journal. Recovery cannot start an absent operation or reclaim a journal superseded by another release. Status checks ownership without treating observation as an apply or rollback request. Mutation still checks the recorded rollback intent. A normal new rollback is a separate operation; this option is for retrying the retained original context.

The lightweight worker tests cover identity preservation, rejection without journal changes, absent/superseded operations and observing a prepared rollback. Rust normalization and historical-input tests pass in the focused suite. CLI help snapshots and schema generation are validated; full gates and real Linux/SSH acceptance remain pending; no end-to-end upgrade acceptance is claimed.

## Original static-worker upgrade coverage

The R36 recheck found a second historical format: the original static worker at `9ec68960` recorded neither `rendered_edge_sha256` nor process observations. Recovery now handles this format only when the pinned host selects static frontends without executable contracts or Linux executable formats. It verifies the installed asset inventory and Caddy as usual, then requires the installed static configuration to equal the original host configuration exactly, with its declared file mode. Any executable-era derived-state field keeps the full derived-state validation path. Removing those fields from an executable journal cannot turn it into a static journal.

The actual original worker now passes 36 deterministic upgrade cases: first release, update and rollback; prepare and payload-free activate retries; and six interruption positions through edge restart. These fixtures use the historical source rather than a rewritten substitute. Their source, hashes and before/after results are retained in the audit evidence. Repository tests cover the original journal shape, input substitutions, partial derived evidence, executable downgrade refusal and refusal to reinterpret a static configuration as a derived route table. Rust CLI/full gates and real Linux acceptance remain pending.

## Hosts without public routes

The checked CLI and worker now carry explicit `null` edge configuration for a selected host with no declared public routes. Every selected unit on that host must have a checked executable contract. The CLI omits Caddy bytes from private-host payloads, while mixed public/private selections still pin and send Caddy to public hosts. `--caddy` and `--caddy-sha256` are a pair required only if at least one selected host is public. Supplying them for an entirely private selection is rejected. The same byte budget applies to the remaining executable/assets payload.

Private hosts use the normal application process lifecycle: exact artifact/configuration checks, candidate startup, process/socket-bound health and readiness, boot enablement, retirement, selected-only history and rollback. The transaction retains two `null` routing metadata files so existing journal/recovery ownership remains consistent; it installs no edge executable or service and does not request the edge account. TLS/tunnel prerequisites and public routes are rejected for such hosts. Cross-host application networking and dependency readiness observations remain separate gaps; the current process listener is loopback only.

Before mutation, Linux preflight rejects an installed, loaded or active edge unit for the same project. A public/private role change also fails the retained route-template check before application processes change. The missing-unit property check follows the properties path of [systemd's `show_one`](https://github.com/systemd/systemd/blob/main/src/systemctl/systemctl-show.c), whose source was inspected; this is not an execution test of the target host's systemd version. Existing-server account installation, systemd behavior and real Linux/SSH acceptance still need validation.

The deterministic worker suite has 81 passing tests. New coverage includes private update/rollback without edge commands, unselected process/history preservation, failed active readiness, twelve interruption points, role-change rejection in both directions, malformed edge inputs, metadata substitution and refusal of existing/active edge units. Rust argument/packaging tests, CLI help snapshots and schema checks now pass. Full gates and real private-host deployment remain pending. No audit item closes from this implementation slice.

## Literal service environment

The selected service's merged `env` map is captured in its format-2 evidence. Verification re-merges exact package metadata and project overrides; a changed setting cannot be accepted by merely replacing the receipt and its convenience checksum. These are public artifact inputs, not secrets: values occur in captured manifests, descriptors, saved plans and generated systemd unit files. No operator environment is read or interpolated. Secret references continue to fail until a separate delivery lifecycle exists.

The release consumer adds `serve.address-env` and rejects a literal setting with the same name, even if the value matches. It reserves enough room for the worker's fresh loopback port. Effective process contracts replace only that listen value. Installed contracts, retries and rollback retain the remaining settings; substitution is refused before activation.

Names follow the existing uppercase configuration convention and are at most 128 bytes. There are at most 128 entries including the listener. Each value is at most 8192 UTF-8 bytes; the combined `name=value` strings plus terminators are at most 32768 bytes. Empty values and Unicode are supported. C0/C1 controls, Unicode noncharacters and BOM are rejected. Newlines and NUL therefore fail explicitly. These bounds apply again in the worker before writes.

Encoding follows [systemd.exec Environment](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html#Environment=), [systemd.syntax quoting](https://www.freedesktop.org/software/systemd/man/latest/systemd.syntax.html#Quoting), and [systemd.unit specifiers](https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html#Specifiers). Double quotes wrap the complete assignment when needed, backslashes and quotes are escaped, and percent signs are doubled. Dollar signs remain literal. Existing simple assignments remain byte-identical so this change does not rewrite units needed for historical recovery.

Focused Python cases exercise encoding, limits, process-contract substitution, request ownership, update and rollback. Rust capture/manifest/contract tests now pass in the focused suite. Real Linux parser and spawned-process environment acceptance remains required; deterministic tests and documentation inspection do not establish it. No source reproduction or runtime attestation claim is added.

## Rust integration acceptance, 8 November 2026

Rebased onto main `442b38b8` after capability-contract PR #630. All 26 retained commits have valid signatures. The rebase preserved the new watch capability checks and omitted two changeset-heading fixes already present on main; graph implementation bytes were unchanged by the rebase.

The first real compile found three callers of a private digest validator. The helper is now crate-visible. Execution also found three stale test fixtures: an absent TOML table key was indexed for mutation, format 2 was incorrectly treated as unsupported, and a consumer metadata test hit an earlier capture filter. Fixtures now reach the intended ownership, unsupported-version and merged-routing checks without weakening production validation.

Passed with pinned nightly `19833af72`:

- 93 focused CLI graph unit tests.
- 149 configuration tests after normal `schema:update`; all three schema aliases add the two expected `address-env` properties.
- 20 graph command and scaffold/help snapshot tests. The one normally ignored real Cargo service case was then run explicitly and passed: two independent feature sets were captured from the same package.

Release plan/apply/status/rollback help is now pinned and describes every input, including independent digests, public-only Caddy and historical recovery. The seven changed/new help snapshots were reviewed before a normal assertion run. The earlier force-pass run only collected candidates; it is not acceptance.

Full workspace gates, Clippy and real Linux/systemd/SSH acceptance remain open. These results do not establish source reproduction, graph approval, dependency readiness, cross-host networking, rootless runtimes or runtime attestation.
