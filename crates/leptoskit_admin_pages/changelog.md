# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-28)

### Features

- **The analytics charts and builders move to the pages crate.** `leptoskit_admin_pages::analytics` now holds the analytics area's drawing: number and date formatting, a query as an address (`params`), the trend, funnel and retention charts, and each builder's controls. The admin's server still runs the queries and draws these pages itself for now; the move lets the next change hydrate them. Nothing on screen changes. _Owner:_ Ifiok Jr. · _Introduced in:_ [5015d6e](https://github.com/leptoskit/leptoskit/commit/5015d6e05a9adb1f67bb78dc7234ed0972b528a2)

#### The analytics builders run as the pages' own Rust

_Owner:_ Ifiok Jr. · _Introduced in:_ [72186b6](https://github.com/leptoskit/leptoskit/commit/72186b64efbc38f26112c16423fe5661dd808290)

Trends, Funnels, Retention and a saved insight are now pages the admin's Wasm client picks up. A builder control changes the page in place: the client asks the same address for the page's model (`x-admin-page: json`), keeps focus and the open Workings drawer where they were, marks the code lines the change changed, says which in the drawer, and scrolls to the first; Back and Forward work the same way. The `+ Add series`, `Breakdown` and `Change` menus are `leptoskit::aria::Menu`s. Save and Add to dashboard ask through the confirm sheet, which can now start a field with a value and offer choices (`Ask::named`, `Ask::choices`, and an `Answer` for what was chosen). A builder opens with its Workings showing from `?workings=<kind>`, and the Ask tabs are a `Tabs` set. The confirm sheet now gives focus back to the control that opened it even when that control never had focus, such as after a pointer press in Safari.

The builders no longer re-read themselves as new events arrive; that comes back with Live's event stream.

- **Dashboards and actors run as the pages' own Rust.** A dashboard, the people behind a point of a chart, and the pages an analytics screen shows instead of its data (connect the service, the service didn't answer, waiting for the first event) are now pages the admin's Wasm client picks up (`Page::Dashboard`, `Page::Actors`, `Page::Notice`). A dashboard and a page waiting for its first event read themselves again in place when the analytics stream has newer events, at most every ten seconds; the waiting page turns into its builder once events exist. _Owner:_ Ifiok Jr. · _Introduced in:_ [1b2dfab](https://github.com/leptoskit/leptoskit/commit/1b2dfab691289e6ebcbf3b3c49d24f08449c4380)
- **A project's Health tab runs as the pages' own Rust.** A project's Health tab is now a page the admin's Wasm client picks up (`Body::Health`), like its other tabs: the health check with its Run button, and the fleet's last hour of host, export and latency charts. The pages crate turns on leptoskit's `apm` feature to draw them. _Owner:_ Ifiok Jr. · _Introduced in:_ [2a3d73d](https://github.com/leptoskit/leptoskit/commit/2a3d73dfcf478704fd3576501b7c98a8ca4eb7a5)

#### Projects, operations and Workings drawers run in Rust

_Owner:_ Ifiok Jr. · _Introduced in:_ [2685e5b](https://github.com/leptoskit/leptoskit/commit/2685e5bfb10ed119600dccfd0316bf9efc0612d4)

The Projects page and each operation's page now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Their Workings drawers are Rust too: opening one moves focus to its Close button, closing it returns focus, `W` toggles it, Escape closes it, and Copy uses the clipboard plugin. Link to an open drawer with `?workings=<id>`.

A running operation is followed live over the admin's own live connection at `/_leptoskit/admin/live`, which only a signed-in admin can open. The old `/infra/operations/{id}/events` endpoint is gone.

- **The admin's Learn hub and lessons run in Rust.** The Learn hub and every lesson page now come from `leptoskit_admin_pages` and hydrate in the browser, like the Glossary. They load no `admin.mjs`. Each lesson is still checked against your project's real state on the server, and the page shows the result. _Owner:_ Ifiok Jr. · _Introduced in:_ [10b163d](https://github.com/leptoskit/leptoskit/commit/10b163d3e4e540eadb7ba50d865228cf9859c10e)

#### Live, the pulse and the builders follow an rpc stream

_Owner:_ Ifiok Jr. · _Introduced in:_ [d2e310d](https://github.com/leptoskit/leptoskit/commit/d2e310da369a6ffa9109362bd7e89cf1ad9dfca5)

The Live page is now a page the admin's Wasm client picks up. The admin's analytics stream is an rpc stream, `ops::analytics_live`, served on the admin's own live connection beside an operation's progress: it sends the pulse about once a second and, on Live, each event as it arrives. Live adds each event at the top (keeping 200), holds them while paused and shows how many wait on `Resume (n)`, narrows the list by name as you type, and opens a row to its properties. The rate card and the top bar's pulse follow the stream on every page. The builders read themselves again when there are newer events, at most every ten seconds, unless a menu, a row, the confirm sheet or a field is open.

`model::Pulse` gains `per_minute`, `reason` and `latest`; all three default when absent.

#### The admin's pages start moving to Rust in the browser

_Owner:_ Ifiok Jr. · _Introduced in:_ [7af0a28](https://github.com/leptoskit/leptoskit/commit/7af0a28a094034a9ef5649ac6fee38e6c4eecf93)

The admin's pages now live in a new crate, `leptoskit_admin_pages`, which builds for the server and for Wasm. `leptoskit_admin` renders a page there from a small model, and the browser hydrates the same component tree, so the page's behaviour is Rust instead of the hand-written `admin.mjs`.

The Glossary is the first page to move. Its theme switch, account menu, search, ⌘K (Ctrl+K elsewhere) to find a screen, and Shift+W for Workings mode all run in Wasm. Every other page still uses `admin.mjs` for now.

Build the client with `leptoskit build --release --platform web` in `crates/leptoskit_admin_pages`, and give the admin its output directory with the new `AdminBuilder::client_dir`, or `lk admin serve --client-dir` (also read from `LEPTOSKIT_ADMIN_CLIENT_DIR`). The admin serves it from `/_leptoskit/admin/pkg/`. Without it, moved pages render but don't respond to presses.

`Environment` now comes from `leptoskit_admin_pages` and is re-exported as before. Parsing one fails with `UnknownEnvironment` instead of `AdminError::Config`.

- **Library, People and a person's page run in Rust.** The analytics Library, the People list and each person's page now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Search and filter build the list's address in Rust. Export asks first, and its confirm button is a download link, so the browser downloads the server's attachment itself. Delete asks for the person's name and keeps its button disabled until the name matches. _Owner:_ Ifiok Jr. · _Introduced in:_ [70f15bb](https://github.com/leptoskit/leptoskit/commit/70f15bbb7593bc69fa743ec4c330fc7a9faac3b7)
- **A project's tabs and actions run in Rust.** A project's Overview, Changes, Releases, Operations, Backups and Config tabs now come from `leptoskit_admin_pages` and hydrate in the browser, with no `admin.mjs`. Their actions ask through a confirm sheet built on leptoskit's `Modal`, which keeps focus inside it and gives it back to the button that asked. Pressing a plan row opens the plan's Workings with the lines that cause it marked. Health is still drawn by the server for now. _Owner:_ Ifiok Jr. · _Introduced in:_ [535aa35](https://github.com/leptoskit/leptoskit/commit/535aa353b341e4e4479d5c9ce19f2b407dbf7b55)
- **Sign-in and refusal run as the pages' own Rust.** The admin's sign-in page, and the page that turns away someone who isn't an admin, are now `Page::SignIn`, drawn without the shell and picked up by the admin's Wasm client. Continue and Switch account start sign-in with `leptoskit::auth::client::sign_in_with_provider` (Switch account asks with `Prompt::Login`), and a main site that can't start says why on the card. _Owner:_ Ifiok Jr. · _Introduced in:_ [3f87203](https://github.com/leptoskit/leptoskit/commit/3f8720317586bf893da312f223e99b8dfd0cc6df)
- **The web overview runs as the pages' own Rust.** The web overview is now a page the admin's Wasm client picks up (`Page::Web`). Its range, compare and filter chips and its table rows change it in place, the way a builder's controls do, and it reads itself again as events arrive. Add to dashboard asks through the confirm sheet. _Owner:_ Ifiok Jr. · _Introduced in:_ [a8220d3](https://github.com/leptoskit/leptoskit/commit/a8220d3b0ace6e1d3f320e5a83112745f408ddee)
