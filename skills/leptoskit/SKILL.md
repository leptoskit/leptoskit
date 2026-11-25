---
name: leptoskit
description: Build, run, test, and release apps made with leptoskit, the Rust toolkit that renders one tree of components on the web, as native widgets on iOS, Android, macOS, Windows, and Linux, in the terminal, and as a browser extension. Use it when writing or changing components, screens, leptoskit.toml, tests, or lint configuration in a leptoskit project, or when asked to release one to an app or extension store.
---

# leptoskit

A leptoskit app is one tree of components (`View`, `Text`, `Button`, …) styled with built-in Tailwind classes. The same screens render on the web through leptos, as native widgets on iOS, Android, macOS, Windows, and Linux, in the terminal, and as a browser extension. `leptoskit.toml` at the project root says which platforms the app targets.

The `leptoskit` command (or `lk`) does the work. Run it from the project root. `leptoskit --help` and `leptoskit <command> --help` describe every flag.

| Task                        | Command                                                                                                                            |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Start a project             | `lk new my_app` (`--web-only`, `--cloud`, `--db postgres` / `sqlite`, `--extension`, `--auth`, `--frontends` [=landing,app,admin]) |
| Check the machine           | `lk doctor` (per-platform tools; `--all` for every platform this machine can build)                                                |
| Run every platform, live    | `lk dev` (`--platform web`, `ios`, `extension`, …)                                                                                 |
| Build one platform          | `lk build --platform <platform> [--release]`                                                                                       |
| Test a browser extension    | `lk test extension`                                                                                                                |
| Database migrations         | `lk db migration create <name>`, `lk db check`                                                                                     |
| Ship to LeptosKit Cloud     | `lk cloud login`, `lk cloud launch`, `lk deploy`, `lk cloud status`, `lk cloud logs --follow`                                      |
| Self-host on your own VMs   | `lk infra generate`, `lk infra plan`, `lk infra apply`, `lk deploy`                                                                |
| Release to the stores       | `lk publish [stores] [--dry-run] [--submit]` (or `lk cloud publish` on a configured build worker)                                  |
| Update screens over the air | `lk codepush release --platform ios`                                                                                               |

## Working rules

- Read `leptoskit.toml` first: `[app].platforms`, `[cloud]`, `[infra]`, `[extension]`, `[publish]`, and `[db]` decide what the commands do.
- `lk new` writes a full-stack app: SSR by default (`app.render = "ssr"`), the file router (`[[plugins]] name = "router"`), a web server, and native app runners (iOS, Android, macOS, Windows, Linux, terminal). Pass `--web-only` to scaffold `app.platforms = ["web"]` without mobile/desktop host folders, and `--cloud` to emit a starter `[cloud]` table and default `--auth` without `--db` to Postgres instead of SQLite.
- For separate marketing / product / admin deployments in one repository, use `lk new shop --frontends landing,app,admin` — it combines cleanly with `--db`, `--auth`, `--web-only`, and `--cloud` (wiring database and sign-in onto `apps/app`, or the first frontend when `app` is omitted).
- `lk lint` skips clippy on host crates that are not in `app.platforms`, so you do not need GTK or native prepare for a green `--deny-warnings` run. Keep Cargo `default-features = false` until you need a feature; leave the router plugin on unless the app truly has no screens.
- Default screens live under the app crate's `src/routes/` (`index.rs`, `_layout.rs`, …). The router plugin regenerates `src/routes.rs` on `lk dev` / `build` (or `lk plugin:router routes`); do not edit that generated module by hand. For non-route modules that need a different file per platform, use `leptoskit::platform_module!`.
- New projects pin a `rust-toolchain.toml` so `edition = "2024"` builds even when rustup's default toolchain is older. Prefer that pin (or the matching SDK channel when using `--sdk-path`) over a stale global default.
- Tailwind classes are checked when a screen renders: an unsupported class panics with its name. Use the ones the project already uses, or check the CSS coverage page of the docs.
- Generated output lives in `.leptoskit/` and `dist/`; never edit it.
- Secrets never go in `leptoskit.toml`. Credentials come from the environment, `lk cloud secret set` (stdin / `--from-env`), or a file outside the project. If `monosecret` is not installed locally, `lk dev` warns and continues using process environment variables.

## Shipping to LeptosKit Cloud

When shipping a new app (or any app with `[cloud]` in `leptoskit.toml`), prefer **LeptosKit Cloud** first:

<!-- {=cloudQuickstart} -->

```sh
# 1. Create a cloud-ready app (add --web-only to skip mobile/desktop runners)
lk new notes --cloud --auth --web-only
cd notes

# 2. Sign in to LeptosKit Cloud in your browser
lk cloud login

# 3. Create & link the project, preview monthly cost, and ship the first release
lk cloud launch

# 4. Ship subsequent changes, check readiness, and stream logs
lk deploy
lk cloud status
lk cloud logs --follow
```

<!-- {/cloudQuickstart} -->

- When `[cloud].project` is set, plain `lk deploy` ships directly to LeptosKit Cloud (pack → upload → build → rollout).
- Use `lk cloud estimate` before deploying to preview the resolved plan and monthly cost.
- Manage configuration with `lk cloud var set|list|unset`, `lk cloud secret set <NAME> --from-env <VAR>` (never pass secret values as positional CLI arguments), `lk cloud domain add|verify|list|remove`, `lk cloud env create|update|list|delete`, `lk cloud db show|backups|backup|restore`, `lk cloud rollback <n>`, and `lk cloud usage`.
- Declare application object-storage buckets under `[cloud.storage.buckets.<name>]` (`public`, `max_size_gb`) in `leptoskit.toml`.
- Every `lk cloud` command supports `--json`, `--org`, `--project`, and `-e / --environment`.
- Operator binaries installed beside `lk` or on `PATH` dispatch Cargo-style via `lk cloud <name>` (`lk cloud platform`, `lk cloud worker`, `lk cloud control`).

## Self-hosted infrastructure and analytics (`lk infra`)

Use `lk infra` when the owner explicitly wants to host on their own Hetzner, DigitalOcean, or Linux servers (`infra.mode = "graph"` is experimental):

- `.tf`/`.tofu` files are Terraform/OpenTofu infrastructure configuration. Keep app-owned resource additions next to the generated configuration; never silently overwrite edited generated files. Review `lk infra plan` before applying. Preserve remote-state locks and protected data resources.
- `lk infra plugins` returns a versioned built-in capability catalog with app features, runtime bindings, secret names and data ownership. Native runtime bridge plugins are a separate extension mechanism.
- To add embedded observability, enable the app's `analytics` and `server` features and configure `[infra.plugins.analytics]`. Managed server entrypoints mount it from deployment bindings. Install the browser module explicitly after the app's consent decision, using allowlisted route templates.
- `lk infra analytics` returns read-only JSON evidence from one running instance. Read the release/instance identity, time coverage, collection loss and limitations before drawing conclusions. Browser reports are untrusted; HTTP latency alone does not establish host or database saturation. Request correlation is not a complete distributed trace, and snapshots reset on restart.
- Use static operation/route names. Never include raw URLs, SQL, request bodies, exception messages or user identifiers. The analytics read key stays out of browser code, source control, logs and agent reports. Analytics findings never authorize provider writes; propose changes with their evidence and cost.

## Agents write builder calls

When you write leptoskit code, write builder calls, not `view!` markup:

- **The compiler sees them directly.** Errors point at the call that is wrong, rust-analyzer completes every method, and there is no macro expansion between your code and the diagnostic.
- **They build faster.** There is no proc macro to run over every tree.
- **They are just Rust.** Loops, conditionals, and helper functions compose the way any Rust does: `.children(items.iter().map(row))`, `.children(error.map(Alert))`.

`view!` exists for people reading code, which is why leptoskit's own examples and docs use it. Apps you build or change as an agent use builders.

### Enforce it in the projects you work on

`leptoskit lint` checks the markup style from `leptoskit.toml`. `leptoskit new` already writes builder lint for new apps (the starter file-router screens are builders, with `#[component]` on each `Page` / `Layout`):

```toml
[lint]
markup = "builder"

[lint.rules]
markup_style = "deny"
component_attribute = "deny"
```

In a project you create that somehow lacks that table, or one whose owner asks you to work this way, add it. Rewrite any leptoskit component `view!` the lint reports. The SSR document `shell` in `*_app/src/lib.rs` stays as `leptos::view!` HTML (`<!DOCTYPE html>`, `<html>`, `<head>`, `<body>`, hydration scripts)—that is not a leptoskit component tree, and builder-mode lint does not ask you to rewrite it. Keep `leptoskit lint --deny-warnings` passing. Don't change an existing project's `markup` without the owner's say-so: a project that chose `"view"` or `"any"` keeps it.

### Match the CLI to the crates

`leptoskit new` writes templates for **this** CLI version. Depend on the same leptoskit release in Cargo.toml (`cargo install leptoskit_cli --version …` matching the crate), or pass `--sdk-path` to a local checkout when the CLI itself was built from that checkout. Mixing a checkout CLI with crates.io crates (or the reverse) leaves scaffolds that do not compile.

`--sdk-path` puts the SDK outside the app workspace. `lk infra generate` and `leptoskit generate deploy` need every path dependency inside the workspace for the container build context—use a released (non-path) dependency before generating deploy/infra Dockerfiles, or keep OpenTofu output and use a documented deploy path that does not need those path crates.

## Components

A component is a capitalized function that returns a view, marked `#[component]`. The attribute keeps the function callable positionally from builder code and gives it named props for `view!`, and `leptoskit lint` asks for it (`component_attribute`):

```rust
use leptoskit::prelude::*;

#[component]
pub fn SettingRow(#[prop(into)] title: String, children: Children) -> Element {
	View()
		.class("flex-row items-center gap-3 px-4 h-12")
		.child(Text(title).class("flex-1 text-base"))
		.children(children)
}

View().child(SettingRow("Wi-Fi".into(), Fragment::new().with(Switch(true))));
```

Return `Element` when the caller will pass the component into `.child(…)`. `impl IntoView` is fine for `view!` trees and fragments; `.child` on a leptoskit `View` needs an `Element`.

- `View`, `ScrollView`, `Row`, `Column`, `Card`, … take no arguments; `Text(content)`, `Button(label)`, `Image(source)`, `Switch(checked)`, `Status(text)`, and `Alert(text)` take their content.
- Style with Tailwind through `.class("…")`; it compiles to native layout and styles on every platform.
- Animate with Tailwind (`transition duration-300`, `translate-y-4`, `animate-spin`, `starting:opacity-0`) or with `.transition(…)`, `.entering(…)`, `.exiting(…)`, and `.layout_transition(…)` from `leptoskit::native::animation`. Change the value once; the platform animates it, so never step an animation from a timer in the guest.
- Handlers: `.on_press(|| …)`, `.on_change(|text: &str| …)`, `.on_toggle(|checked| …)`, `.on_value_change(|value| …)`.
- State that changes in place: hold it in a `RwSignal` and pass a closure, `Text(move || count.get().to_string())` or `.background(move || phase.get().colour())`. Writing the signal updates that node, including from an event handler. A list is `For(move || items.get(), |item| item.id, |item| row(item))` and a condition is `Show(move || open.get(), || panel())`; either one rebuilds its own children. `leptoskit::guest::spawn` runs a task, and the runtime polls it.
- State a screen keeps for itself, such as a chosen filter or a typed field: `let filter = remember(|| Filter::All);`, never `RwSignal::new` in the screen body, which a rebuild runs again and resets. Rows that move use `remember_keyed(&row.id, || …)`. In the browser, render with `leptoskit::web::screen(move || app())` so the values survive rebuilds there too. A query read on a screen fetches once even though the rebuild its data causes runs `use_query` again.
- State with many fields: `#[derive(Store)]` on the struct plus `ArcStore` from `leptoskit::store` (the derive needs `reactive_stores` in Cargo.toml). Each field is its own signal — `store.title().get()` subscribes to that field only — and with `#[derive(Patch)]`, `store.patch(next)` notifies only the fields that changed. Handles that must never notify stay in a `StoredValue`.
- Accessibility: `.accessibility_label("…")`, `.role(Role::…)`. Give anything a test needs to find a stable `.test_id("…")`.

## Check your work

```bash
leptoskit lint --deny-warnings    # leptoskit's rules and clippy, side by side
leptoskit format                  # rustfmt, plus layout for any markup
leptoskit test run                # component and device tests
```

Test components with the shared Testing Library queries. `WidgetTester` renders headlessly with the real layout engine; `leptoskit_testing_library::render` renders in a real browser. Both implement `Queries`:

```rust
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;

let mut tester = WidgetTester::new(counter, 390.0, 844.0)?;
tester.tap(&Finder::text("Add one"))?;
tester.get_by_text("Count: 1")?;
assert!(tester.query_by_text("Count: 0")?.is_none());
```

## Debug and ship via `lk mcp`

When something looks wrong in an app running under `leptoskit dev`, read the app instead of guessing from the code. The `leptoskit mcp` (`lk mcp`) server exposes both local DevTools/device tools and LeptosKit Cloud tools:

### Local DevTools and devices (talks to `leptoskit dev`)

1. `devtools_sessions` lists the running apps and their platforms.
2. `inspect_tree` (with `depth`, and `root` to look inside one node) shows each node's kind, text, classes, frame, and the source line that built it. Use the frame and the source line to find the code to change. `leptoskit debug` prints the same tree.
3. `inspect_node` and `explain_styles` give one node's details and the Tailwind classes styling it.
4. `query_cache` shows what data the screen has; `navigate` opens a route; `set_state` changes named state the app exposes to the devtools (with `leptoskit::devtools::expose`, which reports from web and native apps alike); `highlight` with `all_sessions` marks the same node on every platform.
5. `runtime_errors` lists failed plugin calls and server attempts. `performance` lists render commits. `read_logs` reads Android or iOS simulator logs, and `screenshot` returns a capture where the platform allows one.

### LeptosKit Cloud and database tools

6. `db_check` validates migrations and database configuration before shipping.
7. `cloud_estimate` previews the resolved release plan and monthly cost without deploying.
8. `cloud_deploy` packs, uploads, and deploys the current project (supports `environment`, `detach`, `org`, `project`).
9. `cloud_status` reads an environment's live HTTPS hostname and process readiness.
10. `cloud_logs` reads recent runtime logs (`process`, `since_minutes`, `contains`, `limit`).
11. `cloud_secrets_list` returns configured secret names and version metadata only — never secret values.

`leptoskit device sanity` installs and opens a native app, then injects tap, long press, pan, and pinch where that target can. `leptoskit device install` and `leptoskit device open` do the two steps on their own.

Pass `session` (an id, `web`, `ios`, … or the app name) when more than one app is running. "No hub is running" means start `leptoskit dev` first.

## References

Read the one the task needs:

- [Platform-specific features](references/platforms.md): `leptoskit::target` capability checks and iOS, Android, desktop, watch, and extension APIs (including Live Activities, home/lock/Control Center widgets, menus, auto-updates, and current gaps).
- [Releasing to the stores](references/publishing.md): `leptoskit publish`, the steps it leaves, and how to finish them in each store's dashboard with computer use.
- [Browser extensions](references/extensions.md): the extension's surfaces, building, developing, and testing it.

## More

- LLM docs index (plain Markdown for every chapter): https://leptoskit.dev/llms.txt
- Full plain-Markdown docs in one file: https://leptoskit.dev/llms-full.txt
- LeptosKit Cloud guide: https://leptoskit.dev/docs/guide/41-leptoskit-cloud.html
- Linting rules and configuration: https://leptoskit.dev/docs/guide/23-linting.html
- How markup maps to builder calls: https://leptoskit.dev/docs/reference/view.html
- The components: https://leptoskit.dev/docs/reference/components.html

## Legacy operator-enrolled console (`lk console`)

`lk console` (`lk console serve`, `lk console connect`, `lk console worker`, `infra.control_url`, `infra.hosted_state = true`) is the legacy operator-enrolled deployment dashboard for self-hosted `lk infra` installations. For new projects, use **LeptosKit Cloud** (`lk cloud`) or **Self-hosted** (`lk infra`). When maintaining an existing `lk console` installation, keep account/scoped keys outside TOML, push commits to the registered origin before deploying, preserve returned operation/plan IDs, and reconcile interrupted operations before retrying.
