# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

#### Analytics in the admin

_Owner:_ Ifiok Jr. · _Introduced in:_ [60e11ae](https://github.com/leptoskit/leptoskit/commit/60e11ae75fdffff7d6da8bc30d4369aced2e9d41)

The admin's Analytics area now reads your site's analytics. Connect it with `.analytics(AnalyticsService::new(url, token))` on the builder (or `AnalyticsService::from_env()`, which reads `LEPTOSKIT_SERVICE_ANALYTICS_URL` and `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`). The admin calls the analytics service as the app, signed with the service's token, which stays on the admin's server. Without a service, every Analytics page shows how to connect one.

The pages:

- **Overview**: visitors, pageviews, sessions, and bounce rate against the previous period, plus top and entry pages, referrers, channels, UTM, devices, browsers, and operating systems.
- **Trends**: a builder over the events your app has sent, with a `struct` badge on typed ones, and controls for math, breakdown, interval, compare, and range.
- **Funnels**: conversion and the median time to convert.
- **Retention**: a heat grid of cohorts.
- **People**: search and a property filter. Each person's page shows their timeline, ids, and sessions, with export and delete behind an in-page confirmation.
- **Live**: an event tail with a filter and pause. The top bar's pulse follows it.
- **Library**: saved insights and dashboards that update as events arrive.

Every point, step, drop-off, and retention cell opens onto the people behind it. Every insight's **Workings** drawer follows its number through five stations:

- **Emit**: the Rust type.
- **Send**: the bytes it sends.
- **Store**: the tables it lands in.
- **Ask**: the query as Rust, SQL, and HTTP, updating as you change the builder.
- **Draw**: the rows the chart was drawn from.

Learn's analytics lessons now tick from your real data.

#### An admin for your site

_Owner:_ Ifiok Jr. · _Introduced in:_ [c3f0a63](https://github.com/leptoskit/leptoskit/commit/c3f0a63a4bb79a192f4abb5aabdb45dcfe308222)

`leptoskit_admin` is one web app for looking after a deployed leptoskit site: its infrastructure, its analytics, and short lessons in both. It is its own app with its own address, and people sign in through your site's own sign-in: they press **Continue**, sign in on your site, and come back. Only accounts your site gives the `admin` role (or the roles you pass to `.roles(…)`) get in; anyone else sees who they signed in as, can switch account, and can read how admins are chosen.

Infrastructure shows what's serving, the changes waiting in `leptoskit.toml` with the plan to review, releases, operations, health from your servers' route metrics, backups, and the configuration. Plan, apply, deploy, first release, maintenance, roll back, health checks, and backup verification each ask you to confirm, run on the deployment service's worker under your name, and stream their progress to the page. Analytics teaches until your app sends its first typed event, and Learn's lessons tick themselves when they become true of your projects. Every card has a **Workings** drawer that walks it back from the check, to the command, to the plan, to the generated `main.tf.json`, to the `leptoskit.toml` lines that declare it.

Build one with `Admin::builder(app_name)`, `.main_site(MainSite::new(issuer, client_id, secret))`, `.keys(…)`, `.data_dir(…)`, and `.control(service)`, then serve `admin.router()`. The `admin_demo` example runs a site and its admin side by side.

#### Projects, operations and Workings drawers run in Rust

_Owner:_ Ifiok Jr. · _Introduced in:_ [2685e5b](https://github.com/leptoskit/leptoskit/commit/2685e5bfb10ed119600dccfd0316bf9efc0612d4)

The Projects page and each operation's page now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Their Workings drawers are Rust too: opening one moves focus to its Close button, closing it returns focus, `W` toggles it, Escape closes it, and Copy uses the clipboard plugin. Link to an open drawer with `?workings=<id>`.

A running operation is followed live over the admin's own live connection at `/_leptoskit/admin/live`, which only a signed-in admin can open. The old `/infra/operations/{id}/events` endpoint is gone.

- **The admin's Learn hub and lessons run in Rust.** The Learn hub and every lesson page now come from `leptoskit_admin_pages` and hydrate in the browser, like the Glossary. They load no `admin.mjs`. Each lesson is still checked against your project's real state on the server, and the page shows the result. _Owner:_ Ifiok Jr. · _Introduced in:_ [10b163d](https://github.com/leptoskit/leptoskit/commit/10b163d3e4e540eadb7ba50d865228cf9859c10e)

#### The admin's pages start moving to Rust in the browser

_Owner:_ Ifiok Jr. · _Introduced in:_ [7af0a28](https://github.com/leptoskit/leptoskit/commit/7af0a28a094034a9ef5649ac6fee38e6c4eecf93)

The admin's pages now live in a new crate, `leptoskit_admin_pages`, which builds for the server and for Wasm. `leptoskit_admin` renders a page there from a small model, and the browser hydrates the same component tree, so the page's behaviour is Rust instead of the hand-written `admin.mjs`.

The Glossary is the first page to move. Its theme switch, account menu, search, ⌘K (Ctrl+K elsewhere) to find a screen, and Shift+W for Workings mode all run in Wasm. Every other page still uses `admin.mjs` for now.

Build the client with `leptoskit build --release --platform web` in `crates/leptoskit_admin_pages`, and give the admin its output directory with the new `AdminBuilder::client_dir`, or `lk admin serve --client-dir` (also read from `LEPTOSKIT_ADMIN_CLIENT_DIR`). The admin serves it from `/_leptoskit/admin/pkg/`. Without it, moved pages render but don't respond to presses.

`Environment` now comes from `leptoskit_admin_pages` and is re-exported as before. Parsing one fails with `UnknownEnvironment` instead of `AdminError::Config`.

- **Library, People and a person's page run in Rust.** The analytics Library, the People list and each person's page now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Search and filter build the list's address in Rust. Export asks first, and its confirm button is a download link, so the browser downloads the server's attachment itself. Delete asks for the person's name and keeps its button disabled until the name matches. _Owner:_ Ifiok Jr. · _Introduced in:_ [70f15bb](https://github.com/leptoskit/leptoskit/commit/70f15bbb7593bc69fa743ec4c330fc7a9faac3b7)
- **A project's tabs and actions run in Rust.** A project's Overview, Changes, Releases, Operations, Backups and Config tabs now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Their actions ask through a confirm sheet built on leptoskit's `Modal`, which keeps focus inside it and gives it back to the button that asked. Pressing a plan row opens the plan's Workings with the lines that cause it marked. Health is still drawn by the server for now. _Owner:_ Ifiok Jr. · _Introduced in:_ [535aa35](https://github.com/leptoskit/leptoskit/commit/535aa353b341e4e4479d5c9ce19f2b407dbf7b55)
- **The admin ships no hand-written JavaScript.** `assets/admin.mjs` is gone, with its `{BASE}/assets/admin.mjs` route, the server-sent events at `{BASE}/api/analytics/live`, and the server-drawn screens it ran on. Every admin page is drawn by `leptoskit_admin_pages` and picked up by its Wasm client, and the analytics stream is the `analytics_live` rpc stream on the admin's live connection. _Owner:_ Ifiok Jr. · _Introduced in:_ [baadbbc](https://github.com/leptoskit/leptoskit/commit/baadbbc4d0764fc5a509061fc8c241324e91c921)

#### Honest simulations and Workings you can follow

_Owner:_ Ifiok Jr. · _Introduced in:_ [19fc493](https://github.com/leptoskit/leptoskit/commit/19fc493af2a2b28674f7c64e676c96f7080601fe)

`AdminBuilder::simulated_infrastructure(true)` tells the admin that its control service's worker runs against a stand-in cloud. Every screen then shows **Simulated** beside the environment pill, Overview and Changes say that plans and applies contact no provider, the plan and apply confirmations say they act on the stand-in, and Learn says a plan was applied on the simulated cloud. It's off by default, so a real admin keeps its real-infrastructure wording. `admin_demo` turns it on unless it runs with `--executor lk`.

The Workings drawer is easier to follow:

- Changing the builder while a drawer is open scrolls the drawer (and only the drawer) to the first line that changed, marks its station **changed**, and says what changed under the drawer's title. Focus stays where it was, reduced motion jumps instead of scrolling, and a live refresh never moves the drawer.
- An in-place page update keeps the drawer's scroll position, the stations you opened, and your focus.
- **Copy** sits in a row above each code block, labelled with the language, so it never covers code.
- On phones, stations start as their headers and open when pressed.
- The Ask station's Rust, SQL, and HTTP tabs are real tabs: `aria-selected`, one tab stop, and the arrow keys, Home, and End move between them.
- Escape and Close give focus back to the control that opened the drawer, such as a plan row.

On Live, a typed event always shows its `struct` badge, even when it arrives after an untyped one.

Simulation wording also covers deploy, first release, maintenance, rollback, health checks, and backup verification. Confirmations and lesson evidence distinguish sample results from real work, and simulated plans never claim to execute the operator CLI. The browser CI suite covers Workings, live events, chart colours after builder changes, and confirmation dialog focus.

Releases recorded in the same second keep their journal order, so the current release and rollback choices do not depend on random operation IDs.

Changing a chart's breakdown updates its generated styles immediately, keeping the same series colours as a full reload. Confirmation dialogs keep keyboard focus inside, make the background inert, and return focus to their opener when dismissed; Escape no longer also closes Workings behind the dialog.

Live's Send station measures the newest event fetched for the page, including typed events. It no longer says that no event has arrived when the list already contains events.

#### The public demo's service runtime

_Owner:_ Ifiok Jr. · _Introduced in:_ [19fc493](https://github.com/leptoskit/leptoskit/commit/19fc493af2a2b28674f7c64e676c96f7080601fe) · _Last updated in:_ [108798e](https://github.com/leptoskit/leptoskit/commit/108798e4b4e8c0df3bf25c8bac41bc172500e0e6)

`examples/demo_service` is the public demo's process: one image per UTC day, a daily generation published with a durable marker under kernel file locks, and a same-PID re-exec at midnight. It serves the site, the admin, and a private loopback (health checks and the token-authenticated analytics prefix), refuses every other `Host` with 421, and caps the `/demo/generation` event stream at 1,000 connections. Termination is authoritative: the SIGTERM listener installs before serving and stays armed through the drain and the runtime's shutdown, so a signal that races a midnight reset or a bounded retry — in either order — wins, and the image exits 0 instead of re-executing. Successful operation event streams end after 15 seconds so an overlapping candidate reconnects into the durable terminal state. Origins, the bind, and `Host` headers normalize to one authority representation — lowercase hosts with a single DNS trailing dot, canonically bracketed IPv6, explicit or implicit default ports equal — so misconfigured origins that fold into one authority refuse to start, and requests with a missing, duplicated, or malformed `Host` are refused 421. Visitor onboarding, visual design, and provider deployment are later tasks; operator rules live in `docs/agents/public-demo.md`.

On Linux the service refuses to start while a nonstandard file descriptor survives into it, with no exemption by name anymore: the startup guard closes its own `/proc/self/fd` enumeration handle before inspecting, so a spare open of `/proc/self/fd` itself — or a handle to any file or directory named `fd` — is refused like any other inherited descriptor, and an inspection failure other than a genuinely closed descriptor is fatal rather than silently excused. The Linux-only process proofs live in `examples/demo_service/tests/inherited_descriptors.rs`.

The demo's analytics uses a 5 ms collection window for its full-month seed, preserving committed ingestion and the publication barrier while leaving production analytics defaults unchanged. Five alternating fresh-volume release runs on a warmed macOS host measured a 0.49 s median to readiness; Linux deployment readiness still needs target-host verification.

The service is Unix only, deliberately: its runtime modules, dependencies, and integration tests are gated to `cfg(unix)`, so the workspace still checks on Windows while the binary refuses to start there — a nonzero exit naming the Unix `flock`/`exec` requirement and the Linux deployment, before reading configuration or binding. No Windows emulation exists or is planned; the same-PID exec and kernel locks are the contract.

The demo's front door now works: a visitor mints a browser-owned pass (`__Host-demo-pass`/`demo-pass`, expiring at the next 00:00 UTC, only its hash stored) and gets their own Ada or Grace at a synthetic `@demo.invalid` address. Generation format 2 adds the visitors' tables and the admin client's `roles` scope, so sign-in is real end to end: the email code is gated by pass ownership (`403 not_your_identity` before any challenge, mail, or rate limit), delivered into a bounded in-memory demo inbox (2,000 addresses, 3 messages each, a 10-minute lifetime) whose receipts are recorded durably before success and compared on read, so fresh mail is never reported lost while another process's mail is honestly `lost`; Ada reaches the admin's overview through the real OIDC flow while Grace is refused on its pages and APIs. Caps — 10 new passes per IP per hour and 5,000 per day (about 10,000 people) — are counted and inserted in one immediate transaction, so two service processes on one volume cannot race past them, and existing passes keep signing in when the day is full. `DEMO_CLIENT_IP_HEADER` names the one header the edge overwrites with the visitor's IP; unusable values refuse the request rather than pass as an unlimited caller, and without it only the socket peer is trusted. Notes, checkout, analytics emission, the browser overlap proof, and the edge deployment remain later tasks.

Visitor sign-in now refuses a configured but missing proxy-IP header, and a blank header setting refuses startup. Synthetic addresses have a database uniqueness constraint; malformed or duplicate Origin headers cannot fall back to Fetch Metadata. A dropped inbox delivery records its new receipt so an older retained code is reported unavailable rather than current.

The demo's pages are now real. The front door ("Be Ada or Grace for a day"), sign-in with the demo inbox drawn as the page's underside (Send, Mail, Read, Verify, Session), and Notes' welcome are leptoskit component trees written as `view!` markup in the admin's own colours. Sign-in is the real emailed-code flow: the code appears only in the visitor's private inbox, **Fill in code** copies it, and the states for a wrong code, a rate limit, a pass that is gone, and mail this process no longer holds all say what happened without claiming a deploy did it. A restart between sending and reading keeps the lost-mail explanation; failed sign-out leaves its button usable for another attempt. Notes opens the real admin in a new tab from the configured address; Grace follows the real refusal and can switch to Ada through the site. Every page carries the demo band with the UTC-midnight countdown measured from the server's clock, discloses the shared data and the stand-in cloud, and opens a focus-managed dialog when `/demo/generation` reports a new day; a request made while a generation builds gets a styled, script-free 503 page. `leptoskit_admin::theme_css()` now exposes the admin's `--admin-*` colour custom properties, for the light and dark appearances and `data-lk-appearance`, so a page beside the admin can share them.

Notes is now a real private notebook. A signed-in visitor saves up to 20 notes of up to 280 characters each (`GET` and `POST /demo/notes`, behind the same pass-and-session ownership as the pages, same-origin, a 4 KiB JSON body, and stable refusal codes), kept in a new `demo_notes` table whose policy lets an account read only its own rows, and kept across same-day restarts. Each saved note sends one real typed `note_created` event (word count, the persona, and a pseudonymous `v-` actor, never the text, address, or network details) through the analytics service's own ingestion route, and the answer says whether the service confirmed it: an error, timeout, or partial receipt still saves the note and reports it unconfirmed, and nothing retries. The note and its event are separate transactions, which the docs state. Reading notes sends no event. The page shows the count, the saved notes (as text, never markup), and, after a confirmed save, the exact actor and a link to the admin's Live. Generation format 3 adds the table, so a release starts the day's notebooks empty beside the recent format-2 generation. The pretend checkout is not built.

The front door is now hydrated leptoskit. Its component tree lives in a new crate, `demo_pages` (`examples/demo_service/pages`), which the service renders on the server and `leptoskit build --release --platform web` compiles to the Wasm client the browser attaches. Choosing Ada or Grace is a typed mutation (`POST /rpc/mint`) beside the unchanged form post, and the new-day dialog listens to a resumable stream on the site's live connection; both call the same stores and limits as before. The front door loads no hand-written script. The service now needs `LEPTOS_SITE_ROOT`, the directory that build wrote, and refuses to start without the client in it. Sign-in is hydrated too: the choice, the emailed code and the demo inbox are Rust on both sides, the code is sent and checked with leptoskit's Auth client, and the inbox is a typed query (`/rpc/inbox`). Notes is hydrated as well: the notebook is a `TextArea` that reads and saves through two typed operations, `notes` and `save_note`, behind the same ownership gate as `/demo/notes`, and the account menu is `leptoskit::aria::Menu`. `assets/demo.js` is deleted, and no page carries a hand-written script.

### Fixes

- **The analytics charts and builders move to the pages crate.** `leptoskit_admin_pages::analytics` now holds the analytics area's drawing: number and date formatting, a query as an address (`params`), the trend, funnel and retention charts, and each builder's controls. The admin's server still runs the queries and draws these pages itself for now; the move lets the next change hydrate them. Nothing on screen changes. _Owner:_ Ifiok Jr. · _Introduced in:_ [5015d6e](https://github.com/leptoskit/leptoskit/commit/5015d6e05a9adb1f67bb78dc7234ed0972b528a2)

#### The analytics builders run as the pages' own Rust

_Owner:_ Ifiok Jr. · _Introduced in:_ [72186b6](https://github.com/leptoskit/leptoskit/commit/72186b64efbc38f26112c16423fe5661dd808290)

Trends, Funnels, Retention and a saved insight are now pages the admin's Wasm client picks up. A builder control changes the page in place: the client asks the same address for the page's model (`x-admin-page: json`), keeps focus and the open Workings drawer where they were, marks the code lines the change changed, says which in the drawer, and scrolls to the first; Back and Forward work the same way. The `+ Add series`, `Breakdown` and `Change` menus are `leptoskit::aria::Menu`s. Save and Add to dashboard ask through the confirm sheet, which can now start a field with a value and offer choices (`Ask::named`, `Ask::choices`, and an `Answer` for what was chosen). A builder opens with its Workings showing from `?workings=<kind>`, and the Ask tabs are a `Tabs` set. The confirm sheet now gives focus back to the control that opened it even when that control never had focus, such as after a pointer press in Safari.

The builders no longer re-read themselves as new events arrive; that comes back with Live's event stream.

- **Dashboards and actors run as the pages' own Rust.** A dashboard, the people behind a point of a chart, and the pages an analytics screen shows instead of its data (connect the service, the service didn't answer, waiting for the first event) are now pages the admin's Wasm client picks up (`Page::Dashboard`, `Page::Actors`, `Page::Notice`). A dashboard and a page waiting for its first event read themselves again in place when the analytics stream has newer events, at most every ten seconds; the waiting page turns into its builder once events exist. _Owner:_ Ifiok Jr. · _Introduced in:_ [1b2dfab](https://github.com/leptoskit/leptoskit/commit/1b2dfab691289e6ebcbf3b3c49d24f08449c4380)
- **A project's Health tab runs as the pages' own Rust.** A project's Health tab is now a page the admin's Wasm client picks up (`Body::Health`), like its other tabs: the health check with its Run button, and the fleet's last hour of host, export and latency charts. The pages crate turns on leptoskit's `apm` feature to draw them. _Owner:_ Ifiok Jr. · _Introduced in:_ [2a3d73d](https://github.com/leptoskit/leptoskit/commit/2a3d73dfcf478704fd3576501b7c98a8ca4eb7a5)

#### Live, the pulse and the builders follow an rpc stream

_Owner:_ Ifiok Jr. · _Introduced in:_ [d2e310d](https://github.com/leptoskit/leptoskit/commit/d2e310da369a6ffa9109362bd7e89cf1ad9dfca5)

The Live page is now a page the admin's Wasm client picks up. The admin's analytics stream is an rpc stream, `ops::analytics_live`, served on the admin's own live connection beside an operation's progress: it sends the pulse about once a second and, on Live, each event as it arrives. Live adds each event at the top (keeping 200), holds them while paused and shows how many wait on `Resume (n)`, narrows the list by name as you type, and opens a row to its properties. The rate card and the top bar's pulse follow the stream on every page. The builders read themselves again when there are newer events, at most every ten seconds, unless a menu, a row, the confirm sheet or a field is open.

`model::Pulse` gains `per_minute`, `reason` and `latest`; all three default when absent.

- **Sign-in and refusal run as the pages' own Rust.** The admin's sign-in page, and the page that turns away someone who isn't an admin, are now `Page::SignIn`, drawn without the shell and picked up by the admin's Wasm client. Continue and Switch account start sign-in with `leptoskit::auth::client::sign_in_with_provider` (Switch account asks with `Prompt::Login`), and a main site that can't start says why on the card. _Owner:_ Ifiok Jr. · _Introduced in:_ [3f87203](https://github.com/leptoskit/leptoskit/commit/3f8720317586bf893da312f223e99b8dfd0cc6df)
- **The web overview runs as the pages' own Rust.** The web overview is now a page the admin's Wasm client picks up (`Page::Web`). Its range, compare and filter chips and its table rows change it in place, the way a builder's controls do, and it reads itself again as events arrive. Add to dashboard asks through the confirm sheet. _Owner:_ Ifiok Jr. · _Introduced in:_ [a8220d3](https://github.com/leptoskit/leptoskit/commit/a8220d3b0ace6e1d3f320e5a83112745f408ddee)
