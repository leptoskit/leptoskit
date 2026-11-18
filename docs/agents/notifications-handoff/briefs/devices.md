# Task: device details on push endpoints, and targeting by device

The brief this work was started from, on a Mac, before it moved to cloud sessions. The findings, the scope and what to build still stand. Where it names a gate, follow [the handoff](../readme.md) instead.

You are implementing one slice of the leptoskit notifications service. Do the work yourself; start no subagents.

## Background

leptoskit is a Rust UI toolkit with first-party backend services. `crates/leptoskit_notifications` is its notification service: a durable per-person inbox plus push delivery over APNs, FCM and Web Push. The goal of the current work is feature parity with Firebase Cloud Messaging and the Firebase notifications composer, so an app needs no third party.

Already merged on `main` (commit `93e2a2af050fea905b51e69b98219691552ca047`):

- Sends to explicit recipients, a topic, a condition, or everyone (`SendTarget` in `src/lib.rs`), with a durable batched fanout job (`fanout_send`, batches of `FANOUT_BATCH`).
- A condition language in `src/condition.rs`: the FCM grammar (`'a' in topics && !('b' in topics)`) plus predicates over attributes the app's server sets per person (`user.plan == 'pro'`, `in [...]`, `has()`, numeric comparisons).
- Message options (`src/options.rs`), silent data messages, `send_test`, `send_status`, `validate_only`, scheduled sends (`visible_at`) and `cancel`.
- Push endpoints (`src/push.rs`, table `push_endpoints`): one row per enrolled device or browser, with `platform`, the secret credential, and a fingerprint.
- Client enrollment: `src/browser.rs` and `src/browser/receiver.js` on the web; `src/native.rs` plus `crates/leptoskit_plugin_notifications` (Rust guest, `apple/NotificationsPlugin.swift`, `android/NotificationsPlugin.kt`) on iOS, Android and macOS.

Read these first: `AGENTS.md`, `docs/agents/notifications-service.md`, `docs/src/guide/41-notifications.md`, then the crate.

## The problem this slice solves

Firebase lets a campaign target devices by platform, app version and language. leptoskit can only target people (topics and person attributes). A message like "ask iOS devices below version 2.1 to update" cannot be sent. The service also knows nothing about a device's language or time zone, which the next slice needs for translated text and delivery in each person's local time.

## What to build

### 1. Device details on an endpoint

- `register_push_endpoint` accepts optional device details: `locale` (a BCP 47 tag such as `en-GB`), `time_zone` (an IANA name such as `Europe/London`), and `app_version` (free text such as `2.1.0`). All three are optional and the argument must default, so existing callers and stored requests keep working.
- Store them as nullable columns on `push_endpoints`. Validate length and shape (define constants next to the existing limits in `src/push.rs`). Normalize the locale to a canonical case. Refuse an unknown time zone name only if you can validate it without a new heavy dependency; otherwise validate its shape and say so in the docs. The next slice will need a time zone database on the server, so if you add one now (for example `chrono-tz`), check `cargo deny` accepts it and keep it behind the server-side feature.
- Re-enrolling the same endpoint updates the details. Changing only the details must not rotate the credential generation or disturb in-flight deliveries.
- The details appear in whatever the person's own endpoint listing returns (`EndpointSummary`), and in nothing another person can read.
- Every client sends them automatically, with no code from the app author:
  - Web: `navigator.language` and `Intl.DateTimeFormat().resolvedOptions().timeZone`. A web app has no version unless the app supplies one; give the app author one obvious place to set it, or leave it empty.
  - iOS and macOS: preferred language, current time zone, `CFBundleShortVersionString`.
  - Android: default locale tag, default time zone id, the package's version name.
  - Before adding host code, check whether leptoskit already exposes locale, time zone or app version on native (search the workspace, including `leptoskit_l10n` and the other `leptoskit_plugin_*` crates) and reuse it if it does.
  - The native reconcile logic (`reconcile` in `src/native.rs`) must re-enroll when the details change (for example after an app update or a language change), and must not re-enroll on every launch when nothing changed.

### 2. Device predicates in conditions

Extend the condition language with a `device.` namespace beside `user.`:

- `device.platform` (the values `PushPlatform::as_str` returns)
- `device.locale` (the full tag) and `device.language` (the primary subtag, lowercase, so `device.language == 'fr'` matches `fr-CA`)
- `device.app_version`
- `device.time_zone`

Operators: the ones `user.` predicates already have. For `device.app_version` only, `<`, `<=`, `>`, `>=`, `==` and `!=` compare as versions: split on `.`, compare numeric components left to right, treat a missing component as 0, and ignore anything from the first `-` or `+` onward. A value that isn't a version by this rule makes an ordering comparison false. Write the exact rule in the docs and test its edges (`2.10 > 2.9`, `2.1 == 2.1.0`, `2.1.0-beta`, empty, garbage).

Any other `device.<key>` is a validation error at send time, with a message that names the allowed keys. `validate_only` must report it.

### 3. What a device predicate means (decided; implement exactly this)

A send addresses people. A device predicate is judged per device:

- Evaluate the condition once for each of the person's active (not invalidated) endpoints, with that endpoint's details, and once with no device at all. With no device, a `device.` comparison behaves the way a `user.` comparison behaves when the attribute is missing; keep the two consistent and document it.
- The person is in the audience if the condition holds with no device, or for at least one active endpoint.
- The push goes only to the endpoints for which the condition holds. If the condition never mentions `device.`, every endpoint gets it, exactly as today.
- The inbox row is per person, as today: one row if the person is in the audience.
- `SendTarget::Recipients`, `Topic` and `Everyone` are unchanged. "Everyone on iOS" is the condition `device.platform == 'ios'`.
- A condition that only has device predicates walks the same registry `Everyone` uses.
- The endpoint filter is applied at the point where push deliveries are enqueued, using the endpoint's details at that moment. Find that point; for a scheduled send it may be later than the fanout. The stored send already holds its target.
- Keep the fanout's cost bounded: load the endpoints for a batch of people in one query, not one query per person.

If you find this cannot be implemented as written (for example, because of how deliveries are enqueued), stop and report what blocks it and what you would do instead. Do not invent different semantics silently.

### 4. Migrations

- Add one migration for SQLite (`migrations/`) and one for Postgres (`migrations-postgres/`), generated with the repository's tooling, not written by hand. `docs/agents/notifications-service.md` and the existing migrations show the shape; the earlier slices generated them with `leptoskit db --service notifications migration create <name>` from a scratch project that path-depends on the crate, and checked drift with `leptoskit db --service notifications check`. `registry.txt` lists the order; your timestamps must sort after `20261117T011208_message_options` (SQLite) and `20261117T011225_message_options` (Postgres).
- Never edit an already merged `up.sql`.
- Write a correct `down.sql` and prove it: apply up, down, up on both engines in a test or a scripted check, and show the output.
- Table and index names must stay unique across every leptoskit crate, because single-database apps flatten namespaces. You are adding columns, not tables, but check any new index name.

### 5. Docs (required, same change)

The public guide is for app authors: plain language, problem first, a concrete example, and what happens on each platform. Examples in docs use `view!` markup (see `AGENTS.md`).

- `docs/src/guide/41-notifications.md`: a section on device details (what is collected, when, on which platform, and that it needs no app code), a section on targeting devices (the predicates, the version rule, and the per-device meaning from part 3 with a worked example where one person has an iPhone and an Android phone), and an update to "What it doesn't do yet".
- `crates/leptoskit_notifications/readme.md` and `crates/leptoskit_plugin_notifications/readme.md` where they list features.
- `docs/agents/notifications-service.md`: the engineering note for this slice.
- A changeset in `.changeset/` (copy the format of an existing one; the notifications crate and the plugin crate both change).
- Rustdoc on every new public item.

## Out of scope (do not do these)

- Translated text variants and local-time delivery (the next slice).
- Tap routing, open tracking, statistics, A/B variants.
- The admin app.
- Refactors, dependency bumps, or formatting sweeps outside the files this slice needs.
- `monochange check --fix` (it strips comments from `Cargo.toml`).

## Acceptance: run these and return the real output

1. The notifications crate's whole test suite, the way the repository's `test:cargo` script runs it (read `devenv.nix`), including the Postgres tests, and the browser tests if a Chrome is available. Say plainly which you could not run and why.
2. `devenv shell -- cargo test -p leptoskit_plugin_notifications`
3. `devenv shell -- cargo test -p leptoskit_admin` and `devenv shell -- cargo test -p leptoskit_cli --lib -- jobs:: auth_command::` (these catch cross-crate table collisions).
4. The migration drift check for both engines, and your up/down/up proof.
5. `devenv shell lint:all`
6. Last, on your final tree: `devenv shell lint:push` (clippy, the full test suite, `cargo deny`; it takes a long time). This is the gate for merging, so it must pass on exactly the tree you hand back. If a test outside this slice fails, rerun it alone once; report it as unrelated only with evidence (it also fails on the base commit, or it passes alone).

New tests must cover at least: details stored and updated on re-enrollment without a credential rotation; details visible only to their owner; each predicate and operator; the version rule edges; the per-device meaning (a person with two endpoints where one matches gets one inbox row and one push; a person with no endpoints against a negated device predicate; a person with only non-matching endpoints is not in the audience); a scheduled condition send; an unknown `device.` key refused, also under `validate_only`; and the native reconcile decision when details change or stay the same. Add the Swift and Kotlin changes with whatever unit tests the plugin crate already has for those hosts; say what is only compiled and not run.
