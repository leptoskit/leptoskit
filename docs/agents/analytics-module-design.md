# Design brief: analytics and the admin as installable modules

Written 19 November 2026, against `main` at `baadbbc`. **Status: proposed, waiting for the owner.** No code follows from this until the owner answers the questions at the end.

## The decision this serves

The owner decided that analytics is a separate, optional module: any leptoskit app can install it, and it is never bundled into core or turned on by default. The same goes for the admin ([handover](js-to-wasm-handover.md#decisions-the-owner-has-made), decision 2). This brief records how close the code is to that today, and proposes what to change.

## What exists today

All of this was checked against the code on `main`.

**The analytics service already installs like any service.** It is the `leptoskit_analytics` crate, published at 0.3.0, with `[package.metadata.leptoskit.service]` (`name = "analytics"`, `port = 3110`, `public = true`, `database = true`, `features = ["service"]`). The [product analytics guide](../src/guide/40-product-analytics.md) tells an app to add:

```toml
[services.analytics]
crate = "leptoskit_analytics"
```

`leptoskit dev` and `serve` then build it, give it its own database, and answer for it at `/_leptoskit/services/analytics/`. This is the same mechanism `leptoskit_notifications` uses.

**The clients are features of the same crate.** An app adds `leptoskit_analytics` itself, with `web`, `native`, `server-client`, `derive` or `typed`. With no features the library depends on nothing, and `tests/dependencies.rs` keeps it that way. Nothing in `leptoskit` reaches it.

**The admin is a crate that `lk` always compiles.** `lk admin serve` lives in `crates/leptoskit_cli/src/admin.rs`, and `leptoskit_cli` depends on `leptoskit_admin` unconditionally. The admin connects to analytics through `LEPTOSKIT_SERVICE_ANALYTICS_URL` and `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`. Without them its Analytics area explains how to connect instead of drawing charts. `leptoskit_admin` and `leptoskit_admin_pages` are not on crates.io.

## What is in the way

1. **The name "analytics" means two things.**
   - `leptoskit`'s `analytics` feature is a deprecated alias of `apm`, kept "for one release". `apm` covers route metrics and request correlation in the app's own process.
   - `[infra.plugins.analytics]`, the `leptoskit.analytics` infra plugin and its `LEPTOSKIT_ANALYTICS_*` bindings also configure APM, not product analytics. The plugin's manifest even asks for the application feature `analytics`.
   - Someone who reads "add analytics" can't tell which one is meant.
2. **Calling the service means compiling the service.**
   - The admin depends on `leptoskit_analytics` with `features = ["service"]`, which brings in SQLite, the rpc server, live queries, and every operation's body.
   - It only needs the operations' declarations and their view types: `service::query::{InsightView, DashboardView, EventDefinitionView, TailItem}`, `service::read::{EventView, PersonView, PersonSummary, SessionView}` and `service::tables::Tile`.
   - The [services guide](../src/guide/32-services.md#write-the-services-operations) says the reverse: an app depends on a service's library for its operations, and the bodies sit behind `service`.
   - An app that wants to read its own analytics from its server has the same problem.
3. **Every `lk` install carries the admin, and through it the analytics service.** That is the opposite of "never bundled by default", and it makes `lk` slower to build.
4. **Nothing proves the published install works.**
   - The examples use `leptoskit_analytics` from the workspace. `examples/analytics` even runs a toy service of its own.
   - No test builds `[services.analytics] crate = "leptoskit_analytics"` the way an app outside this repository would.

## Proposal

The module has three parts, and each is opt-in on its own:

| Part              | What it is                                                         | How an app opts in                                              |
| ----------------- | ------------------------------------------------------------------ | --------------------------------------------------------------- |
| The service       | `leptoskit_analytics` built with `service`, run beside the app     | `[services.analytics] crate = "leptoskit_analytics"`            |
| The clients       | capture from the browser, native apps and the server; typed events | a dependency, or `leptoskit`'s `analytics` feature (question 2) |
| The admin's views | the Analytics area of the admin                                    | running the admin with the service's URL and token              |

### 1. Give the operations a feature of their own

Split `leptoskit_analytics`'s `service` feature into two:

- **`ops`:** the operations' declarations and the view types they return, with `leptoskit_rpc/client` and serde only. No database and no bodies.
- **`service`:** `ops` plus the bodies, the database and the binary, as today.

The admin, and any app server that reads its analytics, then depends on `ops` + `insights`. A dependency test, like the one the client core already has, keeps `ops` free of `leptoskit_db` and the rpc server.

### 2. Untangle the name

- Rename the APM infra plugin to `[infra.plugins.apm]` (`leptoskit.apm`), and its bindings to `LEPTOSKIT_APM_*`. Accept the old table and variable names for one release with a warning that names the new ones.
- After the release that still carries it, drop `leptoskit`'s `analytics = ["apm"]` alias.

From then on, "analytics" always means product analytics.

### 3. Optionally, reach the clients through `leptoskit`

Once the alias is gone, `leptoskit`'s `analytics` feature could re-export the clients as `leptoskit::analytics`. It would forward `web`, `guest` and `server` to `leptoskit_analytics`'s `web`, `native` and `server-client`, plus `derive`, the way `3d` forwards to `leptoskit_3d`. It would stay off by default.

The alternative is to keep `leptoskit_analytics` a plain dependency and document that. That is what notifications does.

### 4. Take the admin out of the default `lk`

There are two shapes:

- **(a) An `lk` cargo feature.** `lk admin` sits behind `leptoskit_cli`'s `admin` feature, which is off by default. Without it, `lk admin serve` says how to get it (`cargo install leptoskit_cli --features admin`). This is small, but every admin user rebuilds `lk`.
- **(b) The admin's own binary.** `leptoskit_admin` ships a `leptoskit-admin` binary, installed on its own. `lk admin serve` runs it when it is installed, and otherwise prints the install command. `lk` then compiles neither the admin nor the analytics service. This needs `leptoskit_admin` and `leptoskit_admin_pages` published, and a way to ship the pages' Wasm client with the binary. Today `lk admin serve` reads it from `--client-dir` or a build.

I recommend (b), because it is the only shape where a default `lk` carries no admin at all.

### 5. Prove the install from the outside

- Add an acceptance test that scaffolds an app with `[services.analytics] crate = "leptoskit_analytics"`, pointed at the workspace copy through `path` so it runs offline. It runs `leptoskit dev`, captures an event from the server client, and reads it back through `ops`.
- Add a dependency test that `leptoskit` with default features, and `lk` without the admin, never compile `leptoskit_analytics`.

### Order of work

Each of these is one pull request, and each works on its own:

1. The `ops` feature, with the admin moved onto it.
2. The APM rename with its warnings.
3. The admin out of the default `lk`: (a) or (b).
4. The acceptance and dependency tests.
5. After the next release, drop the `analytics` alias. If question 2 is yes, add `leptoskit::analytics`.

## Questions for the owner

1. **The APM rename.** May `[infra.plugins.analytics]` and `LEPTOSKIT_ANALYTICS_*` become `apm`, with one release of warnings?
2. **`leptoskit::analytics`.** Should `leptoskit`'s `analytics` feature come back as the product analytics clients once the alias is gone, or should apps keep depending on `leptoskit_analytics` directly?
3. **The admin and `lk`.** Should the admin be (a) an off-by-default `lk` feature, or (b) its own binary? (b) means publishing `leptoskit_admin` and `leptoskit_admin_pages`.
4. **Scope.** Is anything else part of "installable" for you? For example, an `lk add analytics` command that writes the `[services.analytics]` table and the client dependency.
