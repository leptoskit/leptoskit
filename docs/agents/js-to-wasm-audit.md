# Replacing `admin.mjs` and `demo.js` with hydrated leptoskit: gap audit

Research for the rule that every interface is leptoskit components compiled to Wasm, with no hand-written JavaScript. Read-only audit of the repository at commit `5c4b52dda` (branch `feat/public-demo`), 2026-11-18. Every claim below was checked in the code; citations are `path:line`.

Files audited:

- `crates/leptoskit_admin/assets/admin.mjs` (1,403 lines), loaded by `crates/leptoskit_admin/src/shell.rs:799-801`, and the state stylesheet it drives (`shell.rs:660-735`).
- `examples/demo_service/assets/demo.js` (1,513 lines), inlined by `examples/demo_service/src/pages/mod.rs:46,191-193`, and the state rules in `examples/demo_service/assets/demo.css` (741 lines).

---

## Key findings

1. **About half of the 114 behaviours need nothing new.** About 55 work today in a hydrated leptoskit app, because they are only state and rendering: delegating events by `data-testid`, cloning template rows, swapping pages through `DOMParser`, and passing data in `aria-description`. Hydration removes these mechanisms entirely. Both scripts mainly re-implement a reactive renderer by hand.
2. **Two features block most of the rest:** an overlay layer (modal dialog, popover and menu with light dismiss; 15 behaviours) and one-shot focus requests with restore (12 behaviours). Every confirmation sheet, the new-day dialog, the account menu, the avatar menu and the builder menus need the first. Every "focus moves here once" step needs the second. Today `focused(true)` is level-triggered and re-asserts itself after every commit (`crates/leptoskit_native/src/element.rs:1530-1539`, `web/mod.rs:261-286`).
3. **The demo's notebook cannot work without a multi-line field.** `aria_multiline` is only an ARIA flag (`element.rs:1400-1405`, `web/semantics.rs:254`). No backend reads it. The web renderer always emits `<input type="text|password">` (`web/mod.rs:973-1001`), and every native backend builds a single-line widget: `NSTextField` with `setUsesSingleLineMode(true)` (`crates/leptoskit_appkit/src/backend.rs:670-674`), `UITextField` (`crates/leptoskit_uikit/src/backend.rs:335-347`), `gtk::Entry` (`crates/leptoskit_gtk/src/backend.rs:702-712`), and `EDIT` with `ES_AUTOHSCROLL` only (`crates/leptoskit_windows/src/backend.rs:1816`). The page currently marks a single-line `<input>` as multi-line (`examples/demo_service/src/pages/notes.rs:316`), which misreports the field to assistive technology until the script swaps it.
4. **Live updates need a stream that doesn't need a database.** `#[rpc::stream]` and `use_stream` already exist, with WebSocket, an SSE fallback (`crates/leptoskit_rpc/src/client/live.rs:2209-2215`) and a guest transport. However, the server side requires the database-backed live socket: `live = ["server", "db", …]` in `crates/leptoskit_rpc/Cargo.toml:160`, and `QueryCx` holds a `Db` (`crates/leptoskit_rpc/src/server/cx.rs:30-33`). The admin's operation progress, its analytics pulse and tail, and the demo's generation watch are all plain SSE today.
5. **Charts are not a gap.** The admin's charts are already Rust `Vector`s (`crates/leptoskit_admin/src/charts.rs:1-2`, `analytics/draw.rs:797`). The script only redraws two sparklines when a pulse arrives (`admin.mjs:754-777`), and a hydrated page does that by rebuilding the `Vector` from a signal.
6. **The claim in `pages/mod.rs:8-10` is only partly right.** It says the web renderer cannot express the sign-in flow, the countdown or the dialog. The sign-in flow is possible today: `leptoskit_auth::client::send_code`/`verify_code` exist (`crates/leptoskit_auth/src/client/mod.rs:528-546`), and `ErrorResponse` carries the error code and `retry_after`. The countdown is also possible today with `leptos::prelude::set_interval` on the web. A portable clock hook is a small gap. Only the modal dialog is a real gap. `notes.rs:6-8` is right about the missing multi-line field.
7. **Key events landed, but the docs still say they didn't.** `on_key` works on the web and in the terminal (`web/mod.rs:1042-1106`). However, `crates/leptoskit_aria/src/lib.rs:59-62` and `docs/src/guide/16-accessible-components.md:61` still say "the renderer has no key events". As a result, `Tabs` still skips the roving tabindex. That work can start now on the web. Native backends still deliver no `on_key` and ignore `focused(true)` (`docs/src/readme.md:137`).
8. **A hydrated leptoskit app is a supported shape, but no example uses it.** `leptoskit` has `hydrate`/`csr` features (`crates/leptoskit/Cargo.toml:92-93`). The project scaffold hydrates leptoskit components behind `Suspense` with the router (`crates/leptoskit_cli/src/new_project/templates/app.rs:17-57,82-88`), and the CLI tests cover SSR plus hydrate (`crates/leptoskit_cli/tests/render.rs:100`). However, the only hydrating example, `examples/islands`, uses raw leptos HTML, not leptoskit components (`examples/islands/src/app.rs`). The demo and the admin would be the first real SSR-plus-hydrate leptoskit apps in `examples/`.

---

## 1. Summary of gaps, ordered by behaviours unblocked

Counts list the inventory IDs from sections 3 and 4 (AD = admin, DM = demo).

- **Bucket:** B is a missing prop or small extension. C is a new framework capability.
- **Size:** S is under about 400 lines across the stack, M is up to about 1,500, and L is larger.

| #   | Gap                                                                                                                                   | Bucket              | Unblocks                                                                 | Web today                                                                                                                                                                           | Native today                                                                                                                                                  | Size                |
| --- | ------------------------------------------------------------------------------------------------------------------------------------- | ------------------- | ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| G1  | **Overlays**: `Modal` and `Popover` (top layer, backdrop, inert background, focus trap, Escape, press-outside dismiss, focus restore) | C                   | **15**: AD-16, 17, 20, 22, 29, 37, 38, 39, 40, 42, 47; DM-08, 09, 48, 55 | `leptoskit_aria::Dialog` gives roles and wiring only, not modal (`crates/leptoskit_aria/src/dialog.rs:17-21`); the class compiler has no `fixed` or `z-*` (`tailwind/mod.rs:53-56`) | none                                                                                                                                                          | L                   |
| G2  | **One-shot focus requests**, with focus restore                                                                                       | B (web), C (native) | **12**: AD-05, 07, 18, 22, 44; DM-10, 31, 32, 34, 38, 53, 55             | `focused(bool)` is level-triggered and fights the user once kept true                                                                                                               | none (`docs/src/readme.md:137`)                                                                                                                               | M                   |
| G3  | **Streams without a database**: `#[rpc::stream]` on a live mount without `db`                                                         | C                   | **5**: AD-23, 31, 32, 33; DM-11                                          | the client exists: `use_stream` with WebSocket and SSE fallback                                                                                                                     | the client exists: guest WebSocket plugin                                                                                                                     | M                   |
| G4  | **Scroll into view**, one-shot, `center`/`nearest`, honouring reduced motion                                                          | C (small)           | **5**: AD-04, 13, 26; DM-34, 38                                          | only numeric `scroll_offset` on a `ScrollView` (`element.rs:1559-1563`)                                                                                                             | the generated macOS, iOS and Android apps apply `scroll_offset`; the backend crates don't (`docs/src/readme.md:130`)                                          | S-M                 |
| G5  | **Roving focus composites** in `leptoskit_aria`: `Tabs`, `RadioGroup`, `Menu`                                                         | B                   | **5**: AD-08, 09; DM-21, 39, 55                                          | `on_key` exists; `Tabs` keeps every tab in the tab order (`crates/leptoskit_aria/src/lib.rs:59-62`)                                                                                 | blocked on native `on_key` (G10b)                                                                                                                             | M                   |
| G6  | **Platform clock hook** (`use_clock`)                                                                                                 | B                   | **5**: AD-14; DM-03, 26, 32, 33                                          | `leptos::prelude::set_interval` works; `leptoskit_use` timers need a host `Clock` signal (`crates/leptoskit_use/src/time.rs:13-19`)                                                 | the timer plugin backs `leptoskit_query::try_sleep` (`crates/leptoskit_query/src/time.rs:159,178`)                                                            | S                   |
| G7  | **Keyboard**: (a) a `meta` modifier with ⌘ shortcuts allowed; (b) app-level shortcuts that fire with nothing focused; native `on_key` | B for a, C for b    | **5**: AD-11, 45, 52, 53; DM-54                                          | `on_key` drops every ⌘ key (`web/mod.rs:1060`); `KeyPress` has no `meta` (`props.rs:1113-1124`); handlers live on the render root, so they miss keys aimed at `<body>`              | no `on_key` at all                                                                                                                                            | S for a, L for b    |
| G8  | **Typed refusals in the rpc client**: status, code, `retry_after`                                                                     | B                   | **4**: DM-14, 28, 48, 49                                                 | `FnError` is five string variants (`crates/leptoskit_rpc/src/lib.rs:396-410`); `rpc::Error` has no rate limit (`operation.rs:133-158`)                                              | same, since it's transport-agnostic                                                                                                                           | S-M                 |
| G9  | **Text input hints**: keyboard type, `read_only`, `max_length`, capitalisation, autocorrect, submit label                             | B                   | **3**: DM-16, 17, 51                                                     | only `placeholder` and `autocomplete` (`web/mod.rs:984-999`)                                                                                                                        | `autocomplete` already reaches AppKit, UIKit and Android (`crates/leptoskit_appkit/src/backend.rs:691`, `crates/leptoskit_android/android/WidgetHost.kt:990`) | S-M                 |
| G10 | **Document title** (and, optionally, URL fragments)                                                                                   | B                   | **2**: AD-06, 24                                                         | the router ignores fragments (`crates/leptoskit_router/src/url.rs:14-23`); leptos_meta `Title` works on the web                                                                     | no window-title API                                                                                                                                           | S                   |
| G11 | **Pre-hydration fallbacks**: a `noscript:` variant and a native form post                                                             | B (optional)        | **2**: DM-58, 59                                                         | raw HTML strings in Rust today                                                                                                                                                      | n/a                                                                                                                                                           | S                   |
| G12 | **Multi-line `TextArea`**                                                                                                             | C                   | **1**, the demo's core feature: DM-43 (plus DM-51 and 54 once it exists) | none                                                                                                                                                                                | none                                                                                                                                                          | M                   |
| G13 | **Labels that name a field focus it** (`<label for>`)                                                                                 | B                   | **1**: DM-19 (3 labels)                                                  | the `aria_labelled_by` relationship exists                                                                                                                                          | Android `labelFor` is available                                                                                                                               | S                   |
| G14 | **Link opens in a new tab**                                                                                                           | B                   | **1**: DM-07                                                             | no `target` on `Pressable`                                                                                                                                                          | native ignores `href` (`element.rs:1189-1195`)                                                                                                                | S                   |
| G15 | **Download link**                                                                                                                     | B                   | **1**: AD-39                                                             | the sharing plugin refuses downloads on the web (`crates/leptoskit_plugin_sharing/src/web.rs:9-12`)                                                                                 | the desktop dialog has `save_file` (`crates/leptoskit_plugin_dialog/src/desktop.rs:152`)                                                                      | S (web), M (native) |
| G16 | **`prompt=login` on provider sign-in**                                                                                                | B                   | **1**: AD-49                                                             | `StartProvider` has no prompt (`crates/leptoskit_auth/src/api.rs:554-561`)                                                                                                          | same                                                                                                                                                          | S                   |
| G17 | **Announcer helper**                                                                                                                  | B (optional)        | **1**: DM-05 (9 announcement sites)                                      | `Status` plus changing text works; the toaster's private trick for repeating text (`components/toast.rs:910-927`)                                                                   | live regions exist; Linux doesn't announce on GTK 4.6 (`docs/src/readme.md:104`)                                                                              | S                   |

The scripts' state stylesheets also rely on CSS that the class compiler rejects, so it can't be expressed as utilities. This is outside the JS count, but the hydrated pages will hit it:

- `position: sticky` (`shell.rs:710`)
- `z-index` (`shell.rs:676-677,720`)
- `white-space: pre` and `overflow-x: auto` on code blocks (`shell.rs:683-686`)
- `::before`/`::after` glyphs (`shell.rs:686-689,711-712`)
- `box-shadow` (`admin.mjs:232`)

G1 removes the need for `z-index` and `fixed`. The other properties should get their own styling issue.

---

## 2. The toolbox that already exists (bucket A)

| Need                              | leptoskit answer                                                                                                         | Where                                                                                                                      |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Hydrated app                      | `leptoskit` `hydrate` feature; `leptos::mount::hydrate_body`; `leptoskit::web::render` inside `Suspense`                 | `crates/leptoskit/Cargo.toml:93`; `crates/leptoskit_cli/src/new_project/templates/app.rs:17-57,82-88`; `web/mod.rs:86-100` |
| SSR/hydration agreement on styles | content-derived style scopes                                                                                             | `crates/leptoskit_native/src/web/mod.rs:7-9,509-553`                                                                       |
| Reactive regions                  | `For`, `Show`, `Element::text_from`, `remember`, `web::screen`                                                           | `crates/leptoskit_native/src/reactive.rs:198,240`; `remember.rs:285`; `web/mod.rs:71`                                      |
| Events                            | `on_press`, `on_change`, `on_toggle`, `on_key`, `.on(NativeEventSlot::Submit, …)` (Enter in an input)                    | `element.rs:1918-1992`; `web/mod.rs:1079-1093`                                                                             |
| ARIA                              | every typed `aria_*`, `tab_index`, `role` → semantic tags                                                                | `element.rs:1249-1483`; `web/semantics.rs`                                                                                 |
| Widgets (behaviour only)          | `Dialog` (non-modal), `DisclosureButton`/`DisclosurePanel`, `Field`, `Tabs` (non-roving), `ToggleButton`                 | `crates/leptoskit_aria/src/lib.rs:66-78`                                                                                   |
| Live regions and toasts           | `Status`, `Alert`, `Toaster`/`WithToasts`                                                                                | `crates/leptoskit_native/src/components.rs:171-181,60-72`                                                                  |
| Styled runs (inline `code`)       | `Text(…).spans(…)`                                                                                                       | `element.rs:1519-1528`                                                                                                     |
| Server calls                      | `#[rpc::query]`/`#[rpc::mutation]`, `use_action` with built-in idempotency keys, same-origin `fetch` carrying the cookie | `crates/leptoskit_rpc/src/client/mod.rs:27,266-280,861`; `client/browser.rs:1-60`                                          |
| Sign-in                           | `send_code`, `verify_code`, `sign_out_mutation`, `sign_in_with_provider`, `ErrorResponse { error, retry_after }`         | `crates/leptoskit_auth/src/client/mod.rs:528-546,719,901`; `api.rs:382`                                                    |
| Routing                           | `Router::from_browser` (history, back and forward), `navigate`, `replace`, query pairs                                   | `crates/leptoskit_router/src/web.rs:22-80`; `router.rs:114-166`; `url.rs:60`                                               |
| Theme                             | `appearance::set`/`get` (`data-lk-appearance`)                                                                           | `crates/leptoskit_native/src/appearance.rs:1-27`                                                                           |
| Clipboard                         | `leptoskit_plugin_clipboard::web::write_text` (and the guest client)                                                     | `crates/leptoskit_plugin_clipboard/src/lib.rs:591-650`                                                                     |
| Media queries (web only)          | `use_media_query`, `use_preferred_dark`, `use_preferred_reduced_motion`, `use_event_listener`                            | `crates/leptoskit_use/src/web.rs:65,102,120,125`                                                                           |
| Throttle and debounce             | `signal_throttled`, `use_debounce_fn`                                                                                    | `crates/leptoskit_use/src/time.rs:122-172`                                                                                 |
| Timers (web)                      | `leptos::prelude::set_interval`/`set_timeout`                                                                            | leptos_dom 0.9 helpers, re-exported in `leptos::prelude`                                                                   |

---

## 3. Inventory: `crates/leptoskit_admin/assets/admin.mjs`

"A" means possible today with the API named. A gap ID means that part needs the gap. "A + G2" means the state is possible today but one part needs G2.

| ID    | Behaviour                                                                                                                                                                                                | Where                                    | Bucket      | leptoskit answer                                                                                                                                        |
| ----- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| AD-01 | JSON POST helper, same-origin credentials, tolerant JSON parse                                                                                                                                           | 12-26                                    | A           | rpc client (`client/browser.rs`) once the endpoints are `#[rpc::mutation]`                                                                              |
| AD-02 | Fire-and-forget preference writes (`workings_mode`, `use_note`, `dismiss_note`)                                                                                                                          | 28-30; 85, 133, 172, 275, 400, 420       | A           | `#[rpc::mutation]` + `use_action`                                                                                                                       |
| AD-03 | Notice line text                                                                                                                                                                                         | 32-37; 398, 428-431, 938, 979, 997, 1029 | A           | `Alert` with reactive text                                                                                                                              |
| AD-04 | Scroll the notice into view after a failed action                                                                                                                                                        | 432                                      | G4          | `scroll_into_view`                                                                                                                                      |
| AD-05 | Open a Workings drawer: root attribute, focus its Close button                                                                                                                                           | 47-58                                    | A + G2      | `Show` from a drawer signal; one-shot focus                                                                                                             |
| AD-06 | Drawer ↔ `#workings=` fragment (write, clear, read on load)                                                                                                                                              | 54, 69-71, 1393-1397                     | G10 / A     | fragment API, or move to `?workings=` (query works today)                                                                                               |
| AD-07 | Close a drawer: clear highlights and status, return focus to the invoker or its tab                                                                                                                      | 60-78                                    | A + G2      | signals; focus restore                                                                                                                                  |
| AD-08 | Ask tabs: select, sync `aria-selected` and roving `tabIndex` across stations, preference                                                                                                                 | 81-96, 1165-1168                         | A + G5      | `leptoskit_aria::Tabs` + signal; roving                                                                                                                 |
| AD-09 | Ask tabs: Arrow, Home and End keys                                                                                                                                                                       | 99-116, 1311-1319                        | G5          | `Tabs` key handling                                                                                                                                     |
| AD-10 | Station collapse on phones (`aria-expanded`)                                                                                                                                                             | 119-124, 1177-1181                       | A           | `DisclosureButton`/`DisclosurePanel`                                                                                                                    |
| AD-11 | Workings mode switch (`aria-pressed`, preference) and Shift+W                                                                                                                                            | 126-134, 1185-1187, 1341-1344            | A + G7      | `ToggleButton`; Shift+W needs app shortcuts                                                                                                             |
| AD-12 | Plan row highlights its linked code lines and rows                                                                                                                                                       | 136-151, 1153-1157                       | A           | classes from a signal                                                                                                                                   |
| AD-13 | Smooth-scroll the first linked line to the centre                                                                                                                                                        | 152                                      | G4          | `scroll_into_view(center)`                                                                                                                              |
| AD-14 | Copy a block or all blocks; the button shows "Copied" for 1.2 s; failure notice                                                                                                                          | 157-176, 1188-1204                       | A + G6      | clipboard plugin; timer (`set_timeout` on the web, `use_clock` portable)                                                                                |
| AD-15 | Confirm sheet content: title, prose with `code` runs, radio choices, labelled text field, status line, danger styling, type-to-confirm gating                                                            | 181-299                                  | A           | `Dialog` wiring, `Text().spans`, `Radio` + `RadioGroup`, `TextInput`, `Status`, `enabled(…)`                                                            |
| AD-16 | Confirm sheet layering: a full-screen veil above the page (`absolute; inset:0; z-index:50`)                                                                                                              | 202-204, 299-300                         | G1          | `Modal`                                                                                                                                                 |
| AD-17 | Confirm sheet modality: background `inert`, Tab and Shift+Tab trap                                                                                                                                       | 301-304, 309, 338-354                    | G1          | `Modal`                                                                                                                                                 |
| AD-18 | Confirm sheet initial focus and focus restore to the opener                                                                                                                                              | 305, 310-312                             | G2 (+G1)    | `Modal::initial_focus`, automatic restore                                                                                                               |
| AD-19 | Confirm sheet keys: Enter confirms, Escape cancels, keys don't reach page shortcuts                                                                                                                      | 320-337                                  | A (web)     | `Submit` event; `on_key` returning `Handled` stops propagation (`web/mod.rs:1068-1071,1121-1123`)                                                       |
| AD-20 | Backdrop press cancels                                                                                                                                                                                   | 325-329                                  | G1          | `Modal::on_dismiss`                                                                                                                                     |
| AD-21 | Project ID from the path; action request built from the test ID                                                                                                                                          | 361-388                                  | A           | route params; closures                                                                                                                                  |
| AD-22 | Run an action: version required (focus the field), confirm, busy label, idempotent POST, follow `href`, restore on failure                                                                               | 390-433                                  | A + G1 + G2 | `use_action`, `Router::navigate`                                                                                                                        |
| AD-23 | Operation progress over SSE: de-duplicate by sequence, append rows, settle the previous row, live text, reload on `done`, error text                                                                     | 437-496                                  | G3          | `use_stream`; rows through `For`                                                                                                                        |
| AD-24 | Builder links swap in place: intercept same-path clicks, fetch HTML, `DOMParser`, replace main and aside, keep drawer, scroll, stations and focus, copy stylesheets and render scope, title, `pushState` | 504-745, 1106-1119                       | A + G10     | the router plus an rpc query keyed on the query string; the mechanism disappears; title through G10 (leptos_meta on the web today)                      |
| AD-25 | Wash the code lines and stations that changed after a swap                                                                                                                                               | 687-711                                  | A           | compare previous and next data in Rust (`remember`)                                                                                                     |
| AD-26 | Reveal a change: open its station, status "X changed: …", scroll the drawer only to the line if out of view, reduced motion                                                                              | 592-629                                  | A + G4      | `scroll_into_view(nearest)`                                                                                                                             |
| AD-27 | Back and forward re-swap                                                                                                                                                                                 | 747-751                                  | A           | `Router::from_browser` popstate (`router/src/web.rs:59-80`)                                                                                             |
| AD-28 | Builder menus: one open at a time                                                                                                                                                                        | 729-737, 1278-1283                       | A           | signal                                                                                                                                                  |
| AD-29 | Press outside or Escape closes the builder menus                                                                                                                                                         | 1120-1128, 1321-1326                     | G1          | `Popover::on_dismiss` (Escape alone works today when focus is inside)                                                                                   |
| AD-30 | Redraw both sparklines from a pulse                                                                                                                                                                      | 754-777                                  | A           | rebuild `analytics::draw::sparkline` (`draw.rs:797`) from a signal; draw the end dot inside the `Vector` so each value doesn't mint a `left-[x%]` class |
| AD-31 | Live pulse over SSE: rate, `aria-label`, dot colour, per-minute count                                                                                                                                    | 850-877                                  | G3          | `use_stream`                                                                                                                                            |
| AD-32 | Re-read page data when events arrive, at most every 10 s, not mid-interaction                                                                                                                            | 878-894                                  | A + G3      | query invalidation + `signal_throttled`; the guards become app state, and DOM reconciliation keeps focus anyway                                         |
| AD-33 | Live tail over SSE: clone the template row, set time, labels, typed badge and pretty JSON, cap at 200                                                                                                    | 795-848, 896-907                         | G3          | `use_stream`; `For`; `serde_json::to_string_pretty`                                                                                                     |
| AD-34 | Pause and resume the tail with a queued count                                                                                                                                                            | 1264-1277                                | A           | signals                                                                                                                                                 |
| AD-35 | Filter the tail by event name                                                                                                                                                                            | 784-793, 1366-1371                       | A           | signal + filtered `For`                                                                                                                                 |
| AD-36 | Insight identity carried in `aria-description`                                                                                                                                                           | 910-917                                  | A           | closures capture it; the trick disappears                                                                                                               |
| AD-37 | Save insight: confirm with a name, POST, navigate                                                                                                                                                        | 919-942                                  | A + G1      | rpc mutation                                                                                                                                            |
| AD-38 | Add to dashboard: list dashboards, radio choices (with "new" enabling a name field), POST, navigate                                                                                                      | 944-983                                  | A + G1      | rpc query + mutation                                                                                                                                    |
| AD-39 | Export a person: confirm, fetch JSON, Blob download with a file name                                                                                                                                     | 985-1005                                 | G15 + G1    | `href(…).download(…)`                                                                                                                                   |
| AD-40 | Delete a person: type their name, danger confirm, POST, navigate                                                                                                                                         | 1007-1034                                | A + G1      | `TextInput` + `enabled`                                                                                                                                 |
| AD-41 | People search from three fields; Enter submits                                                                                                                                                           | 1036-1049, 1261-1263, 1385-1390          | A           | `Router::navigate` with a query; `Submit`                                                                                                               |
| AD-42 | Quick search: targets from the rail and lessons, filtered as you type, results listbox over the content                                                                                                  | 1053-1102, 1359-1365                     | A + G1      | targets from the route table; `Show`/`For`; the results layer is a `Popover`                                                                            |
| AD-43 | Enter opens the first result                                                                                                                                                                             | 1374-1384                                | A           | `Submit`                                                                                                                                                |
| AD-44 | "Search" button reveals the box and focuses it                                                                                                                                                           | 1290-1296                                | A + G2      | `Show`; one-shot focus                                                                                                                                  |
| AD-45 | ⌘/Ctrl+K focuses search from anywhere                                                                                                                                                                    | 1306-1310                                | G7          | app shortcut with `meta`                                                                                                                                |
| AD-46 | Theme toggle: resolve the current theme (including `prefers-color-scheme`), set `data-lk-appearance`, persist                                                                                            | 1205-1214                                | A           | `appearance::get`/`set`; persist through the preference mutation (the server keeps writing the root attribute at render, `shell.rs:777-779`)            |
| AD-47 | Avatar menu: toggle; closes on outside press and Escape                                                                                                                                                  | 1215-1218, 1135-1139, 1327-1328          | A + G1      | `Popover`                                                                                                                                               |
| AD-48 | Sign out                                                                                                                                                                                                 | 1219-1222                                | A           | `sign_out_mutation`                                                                                                                                     |
| AD-49 | Continue with the main site, or switch account (`prompt=login`); inline error                                                                                                                            | 1223-1248                                | A + G16     | `sign_in_with_provider`                                                                                                                                 |
| AD-50 | Dismiss a margin note and remember it                                                                                                                                                                    | 1169-1174                                | A           | `Show` + mutation                                                                                                                                       |
| AD-51 | Expand a row's "how" panel                                                                                                                                                                               | 1284-1289                                | A           | Disclosure                                                                                                                                              |
| AD-52 | `w` opens or closes the drawer for the focused or first card                                                                                                                                             | 1334-1357                                | G7          | app shortcut (works on the web today only while focus is inside the root)                                                                               |
| AD-53 | Escape cascade: menus, then avatar menu, then drawer                                                                                                                                                     | 1320-1333                                | A + G7      | root `on_key`; G7 when nothing is focused                                                                                                               |
| AD-54 | Note the first three plan-workings views                                                                                                                                                                 | 1398-1401                                | A           | mutation on mount                                                                                                                                       |
| AD-55 | State stylesheet keyed to root attributes and test IDs                                                                                                                                                   | `shell.rs:660-735`                       | A           | `Show` and classes from signals (see the CSS note in section 1)                                                                                         |

## 4. Inventory: `examples/demo_service/assets/demo.js`

| ID    | Behaviour                                                                                                                                 | Where                                                      | Bucket                 | leptoskit answer                                                                                                                 |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- | ---------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| DM-01 | State handed over in `<script type="application/json" id="demo-state">`                                                                   | 14-17; `pages/mod.rs:189-193`                              | A                      | hydration props and resources                                                                                                    |
| DM-02 | Server-clock offset (`nowMs` + `performance.now`)                                                                                         | 19, 30                                                     | A                      | server `now_ms` + monotonic elapsed time                                                                                         |
| DM-03 | Band countdown, "late" warning, one tick a second                                                                                         | 33-58, 113-137                                             | A (web) / G6           | `set_interval` on the web; `use_clock` portable                                                                                  |
| DM-04 | Band disclosure                                                                                                                           | 139-146                                                    | A                      | Disclosure                                                                                                                       |
| DM-05 | Polite announcer that can repeat identical text                                                                                           | 63-75                                                      | A / G17                | `Status` + reactive text; helper optional                                                                                        |
| DM-06 | Route disclosure                                                                                                                          | 152-160                                                    | A                      | Disclosure                                                                                                                       |
| DM-07 | Admin links open in a new tab, `rel=noopener`, label suffix                                                                               | 163-172                                                    | G14                    | `href(…).target(LinkTarget::NewTab)`                                                                                             |
| DM-08 | New-day dialog shows and hides                                                                                                            | 177-230                                                    | A + G1                 | `Modal` (the page already sets `aria_modal=true`, `pages/chrome.rs:391-392`, which is only true while the script contains focus) |
| DM-09 | Dialog modality: inert siblings, Tab trap, Escape                                                                                         | 187-219, 226                                               | G1                     | `Modal`                                                                                                                          |
| DM-10 | Dialog focus: the title on open, back to the opener or `#main`                                                                            | 195-196, 224-227                                           | G2                     | `Modal` restore                                                                                                                  |
| DM-11 | Generation watch over SSE: compare the day, show the dialog, back off from 2 s to 30 s, close on error, `pagehide`/`pageshow`             | 241-299                                                    | G3                     | `use_stream` (reconnect and backoff are in the live client)                                                                      |
| DM-12 | Notice element: tone glyph, `alert`/`status` role                                                                                         | 304-313                                                    | A                      | `Alert`/`Status` + `Show`                                                                                                        |
| DM-13 | Mint a person: POST `/demo/pass`, follow the redirect                                                                                     | 320-340                                                    | A                      | `#[rpc::mutation]` returning the next URL; navigate                                                                              |
| DM-14 | Mint refusals: 429 with `Retry-After` wording, 503 "full" with countdown, 503 "opening"                                                   | 342-376, 98-109                                            | G8                     | typed refusals (or encode them as `Ok` variants today)                                                                           |
| DM-15 | Front door buttons: busy, notices, announce, stay disabled when full                                                                      | 379-404                                                    | A                      | signals                                                                                                                          |
| DM-16 | Code field: `name`, `maxlength`, `inputmode`, `autocapitalize`, `autocorrect`, `spellcheck`, `enterkeyhint`                               | 436-444                                                    | G9                     | text input hints                                                                                                                 |
| DM-17 | Email field: `readonly`, `type=email`, `name`                                                                                             | 445-449                                                    | G9                     | `read_only`, `Keyboard::Email` (markup sets only `aria_read_only`, `pages/sign_in.rs:325`)                                       |
| DM-18 | Error element gets an `id` for `aria-describedby`                                                                                         | 450                                                        | A                      | `.id(…)`                                                                                                                         |
| DM-19 | Labels focus their fields when clicked                                                                                                    | 453-463, 1431                                              | G13                    | labels render as `<label for>`                                                                                                   |
| DM-20 | Station tabs: select, `aria-selected`, explanation text, `aria-labelledby`                                                                | 467-482, 510-516                                           | A + G5                 | `Tabs` + signal                                                                                                                  |
| DM-21 | Station tabs: Arrow (including Up and Down), Home, End                                                                                    | 496-509                                                    | G5                     | `Tabs` roving                                                                                                                    |
| DM-22 | Station progress lights and the `--fill` bar                                                                                              | 485-494                                                    | A                      | classes and typed width from a signal                                                                                            |
| DM-23 | Show and clear notices; reset `aria-invalid` and `aria-describedby`                                                                       | 520-541                                                    | A                      | `Show`, `leptoskit_aria::Field`                                                                                                  |
| DM-24 | Mailbox: newest first, metadata, code extraction, Fill button, "replaced" note, lost-code card, listening states                          | 546-664                                                    | A                      | `For`/`Show`                                                                                                                     |
| DM-25 | Send and verify controls: disabled while busy or cooling down, labels, help text, root `data-busy`                                        | 666-687                                                    | A                      | signals                                                                                                                          |
| DM-26 | Cooldown ticker                                                                                                                           | 689-701                                                    | A (web) / G6           | as DM-03                                                                                                                         |
| DM-27 | Flow state machine (choose, ready, sent, lost) shown through `html[data-state]` CSS                                                       | 703-712; `demo.css:261-274`                                | A                      | `Show` on a signal                                                                                                               |
| DM-28 | Read the inbox: GET no-store; 401 "pass gone" or "new day"; filter, sort, lost code                                                       | 720-777                                                    | A + G8                 | `#[rpc::query]`; the two 401 kinds need typed refusals                                                                           |
| DM-29 | Pass gone: disable controls, alert, "Start again" link                                                                                    | 779-791                                                    | A                      | signals                                                                                                                          |
| DM-30 | Send and verify through Auth's routes                                                                                                     | 795-810, 866, 914-917                                      | A                      | `send_code()`, `verify_code()`                                                                                                   |
| DM-31 | Auth refusals: `not_your_identity`, `rate_limited` with retry wording, `invalid_code` (invalid flag, `describedby`, focus the field), 503 | 812-858                                                    | A + G2                 | `ErrorResponse.error` and `.retry_after`; focus                                                                                  |
| DM-32 | Send a code: 30 s cooldown, find the new message, highlight it, announce, clear and focus the code field                                  | 860-895                                                    | A + G6 + G2            |                                                                                                                                  |
| DM-33 | Verify: empty check; on success light the stations, announce, navigate after 350 ms unless reduced motion                                 | 897-933                                                    | A + G6                 | `use_preferred_reduced_motion` (web) + timer                                                                                     |
| DM-34 | Fill in code: set the value, clear notices, announce, scroll Sign in into view, focus it                                                  | 937-951                                                    | A + G4 + G2            | controlled `value`                                                                                                               |
| DM-35 | Enter in the code field verifies; typing clears the invalid flag and re-lights the stations                                               | 958-975                                                    | A                      | `Submit`, `on_change`                                                                                                            |
| DM-36 | Check for mail                                                                                                                            | 976-984                                                    | A                      | query refetch                                                                                                                    |
| DM-37 | Narrow screens get "Open your demo inbox", wide screens get help text (`matchMedia`)                                                      | 988-994                                                    | A                      | responsive classes (`max-lg:hidden`, `lg:hidden`)                                                                                |
| DM-38 | Peek link: scroll the inbox into view, focus its first action                                                                             | 995-1006                                                   | G4 + G2                |                                                                                                                                  |
| DM-39 | Persona radio cards: pick, `aria-checked`, roving `tabIndex`, Arrow, Home and End, "Continue as" label                                    | 1010-1055, 1083-1085                                       | A + G5                 | `RadioGroup` (rich cards, so not the bare `Radio` control)                                                                       |
| DM-40 | Choose or swap a person                                                                                                                   | 1057-1076                                                  | A                      | mutation                                                                                                                         |
| DM-41 | First paint; load the inbox when mail is held; clear the interval on `pagehide`                                                           | 1080-1089                                                  | A                      | owner cleanup                                                                                                                    |
| DM-42 | Sign out; failure toast                                                                                                                   | 1095-1112, 1478-1486                                       | A                      | `sign_out_mutation`, `Toaster`                                                                                                   |
| DM-43 | Swap the `TextInput` for a `<textarea>` copying its attributes (`rows=4`, `autocomplete=off`)                                             | 1129-1139                                                  | G12                    | `TextArea`                                                                                                                       |
| DM-44 | Character counter in Unicode scalars, over-limit flag                                                                                     | 1124, 1165-1169                                            | A                      | `text.trim().chars().count()`                                                                                                    |
| DM-45 | Field error wiring                                                                                                                        | 1172-1180                                                  | A                      | `Field`                                                                                                                          |
| DM-46 | Show and clear save results                                                                                                               | 1182-1194                                                  | A                      | `Show`                                                                                                                           |
| DM-47 | Notes list, empty state, count; a full notebook disables the field and Save and announces once                                            | 1196-1225                                                  | A                      | `For`/`Show`                                                                                                                     |
| DM-48 | Signed out versus new day (dialog, sign-in link)                                                                                          | 1228-1249                                                  | A + G1 + G8            |                                                                                                                                  |
| DM-49 | Load and save notes; 401, 409 `notes_full`, 422 empty or too long                                                                         | 1251-1307, 1350-1384                                       | A + G8                 | rpc query and mutation                                                                                                           |
| DM-50 | Validate the draft before sending                                                                                                         | 1282-1290                                                  | A                      |                                                                                                                                  |
| DM-51 | Saving state: disabled button, "Saving…", `aria-busy`, read-only field                                                                    | 1309-1316                                                  | A + G9                 | `read_only`                                                                                                                      |
| DM-52 | Saved: prepend, clear, "new" mark, analytics confirmed or not, actor, announce                                                            | 1319-1347                                                  | A                      |                                                                                                                                  |
| DM-53 | Give focus back after the disabled button dropped it                                                                                      | 1398, 1406-1413                                            | G2                     | one-shot focus                                                                                                                   |
| DM-54 | Ctrl/⌘+Enter saves                                                                                                                        | 1421-1426                                                  | A (Ctrl) + G7a (⌘)     | `on_key` drops ⌘ today (`web/mod.rs:1060`)                                                                                       |
| DM-55 | Account menu: opening focuses the first item; Up and Down; Escape and Tab close; focus returns to the toggle; outside press closes        | 1437-1476                                                  | G1 + G5 + G2           | `Popover` + `leptoskit_aria::Menu`                                                                                               |
| DM-56 | Switch account: sign out, then mint the other person                                                                                      | 1488-1500                                                  | A                      |                                                                                                                                  |
| DM-57 | Page dispatch by `S.page`                                                                                                                 | 1503-1512                                                  | A                      | router                                                                                                                           |
| DM-58 | `data-js` flag script and `html:not([data-js])` CSS                                                                                       | `pages/mod.rs:180-182`; `demo.css:129-143,284-285,570-574` | flag (section 6) / G11 | CSS `@media (scripting: none)`                                                                                                   |
| DM-59 | `<noscript>` mint form                                                                                                                    | `pages/front.rs:467-478`                                   | flag (section 6) / G11 | keep it server-handled                                                                                                           |

---

## 5. Proposed APIs

Each proposal names the builder API, the recommendation, and what each backend needs. **AppKit** is the reference, followed by UIKit, Android, GTK and Windows. The generated Swift and Kotlin renderers are `crates/leptoskit_bridge_ffi/desktop/MacWidgetRenderer.swift`, `mobile/MobileWidgetControls.swift` and `mobile/NativeWidgetRenderer.kt`. They and the binary wire (`crates/leptoskit_bridge_ffi/src/app/binary.rs`) need the same props; `autocomplete` already travels that whole path and is the template for new text-field props.

### G1. Overlays: `Modal` and `Popover` (C, L)

The problem: confirmation sheets, the new-day dialog and menus need to sit above everything, block the page, keep focus, dismiss on Escape or an outside press, and give focus back. `leptoskit_aria::Dialog` deliberately isn't modal "because the renderer cannot contain focus yet" (`dialog.rs:17-21`).

```rust
// A confirmation sheet
Modal(open.get())
    .on_dismiss(move || open.set(false))   // Escape, backdrop press, Android Back, iOS swipe-down
    .initial_focus("confirm-input")         // otherwise the first focusable element
    .child(
        dialog.container(true)
            .child(dialog.title_element(2))
            .children(dialog.description_element())
            .child(form),
    )

// The account menu, anchored to the element with id "me"
Popover("me", menu_open.get())
    .placement(Placement::BelowEnd)
    .on_dismiss(move || menu_open.set(false))   // press outside, Escape
    .child(menu.items(...))
```

**Recommendation:** add two new element kinds, `NativeKind::Modal` and `NativeKind::Popover`, rather than props on `View`. Presenting one creates a different native object (a sheet, popover or window), and kinds are fixed when a widget is created. The surface should treat each overlay subtree as its own layout root. `leptoskit_aria::Dialog` gains `.modal()`, which wraps the dialog in `Modal`, and the renderer emits `aria-modal` itself, so apps can no longer set it without containment as `pages/chrome.rs:391-392` does now. Focus restore is automatic: remember the focused element when the overlay opens and restore it on close.

How each platform presents it:

- **Web.** `Modal` renders `<dialog>` and calls `showModal()` while open. That gives the top layer (so `z-index` and `fixed` are unnecessary), an inert background, Escape through the `cancel` event, and focus moving in. A press on the `<dialog>` element itself is a backdrop press. When SSR renders a dialog that starts open, it writes `<dialog open>`, and hydration upgrades it to `showModal()`. `Popover` uses `popover="auto"` and `showPopover()`, which gives light dismiss and Escape for free, positioned from the anchor's rectangle.
- **AppKit.** `Modal` is a sheet (`NSWindow.beginSheet`) on an `NSPanel` hosting the subtree. `Popover` is an `NSPopover` relative to the anchor view's bounds.
- **UIKit.** `Modal` is a hosting `UIViewController` presented `.formSheet` or `.pageSheet`, with `accessibilityViewIsModal = true` and the presentation delegate reporting swipe dismissal. `Popover` uses `UIPopoverPresentationController`, which becomes a sheet on compact width.
- **Android.** `Modal` is an `android.app.Dialog` whose `setOnCancelListener` calls `on_dismiss` for Back and outside touches. `Popover` is a `PopupWindow` with `isOutsideTouchable` set.
- **GTK.** `Modal` is a `gtk::Window` with `set_modal(true)` and `set_transient_for(parent)`. `Popover` is a `gtk::Popover` parented to the anchor.
- **Windows.** `Modal` is an owned popup window with `EnableWindow(owner, FALSE)`, using the modeless-modal pattern, and `IsDialogMessage` for Escape and Tab. `Popover` is an owned `WS_POPUP` window that closes on `WM_ACTIVATE` inactive.
- **Terminal.** The overlay draws last, on top.

Land the API, the web, the terminal and AppKit first, then the other backends in a stack.

### G2. One-shot focus requests (B on the web, C natively, M)

The problem: `focused(true)` means "keep focus here after every commit" (`element.rs:1530-1539`). Focus that should move once, such as returning to the menu toggle, reaching the code field after a send, or arriving on Sign in after Fill, would snap back on the next unrelated commit once the person has moved on.

```rust
let to_code = FocusRequest::new();             // Copy handle, cheap
TextInput().id("code").focus_on(&to_code);
// later, in a handler:
to_code.request();                             // moves focus once, after the next commit
```

The web follows the existing pattern for applying a target once. `data-lk-scroll` is compared with `data-lk-scrolled` so that a target applies only once (`web/mod.rs:207-259`); focus does the same with `data-lk-focus-req="{epoch}"` and `focus({ preventScroll })`. Keep `focused(bool)` for real level-triggered cases such as a chat prompt.

Natively, nothing moves focus today (`docs/src/readme.md:137`):

- **AppKit:** `window.makeFirstResponder(view)`
- **UIKit:** `becomeFirstResponder()` for text fields, otherwise `UIAccessibility.post(.layoutChanged, argument: view)`
- **Android:** `requestFocus()`, with `isFocusableInTouchMode` for non-text views, plus `sendAccessibilityEvent(TYPE_VIEW_FOCUSED)`
- **GTK:** `grab_focus()`
- **Windows:** `SetFocus(hwnd)`

The same work makes `focused(true)` honest on native.

### G3. Streams without a database (C, M)

The problem: three feeds stream server state (operation progress, the analytics pulse and tail, and the demo day). The framework has the right client in `use_stream`, with WebSocket and SSE fallback in the browser and the WebSocket plugin in guests. However, `#[rpc::stream]` bodies run on the database-backed live socket: `StreamOperation::pump` is gated on `postgres`/`sqlite` (`crates/leptoskit_rpc/src/stream.rs:49-55`), and `QueryCx` holds a `Db`.

```rust
#[rpc::stream]
pub async fn operation_progress(cx: &rpc::StreamCx, id: OperationId)
    -> rpc::Result<impl Stream<Item = Progress>> { … }

// any client
let progress = rpc::use_stream(move || OperationProgress { id });
```

**Recommendation:**

- Add a `StreamCx` that holds only the caller's identity.
- Split the `live` feature into `live-streams`, which needs no `db`, and `live` (`live-streams` plus `db` and queries).
- Add a `Live::streams_only()` mount that a server without a database can merge.

A stream that needs the database keeps `QueryCx`. The demo's bounded-window reconnect (`examples/demo_service/src/admin.rs:9-20`) maps onto `#[rpc::stream(resumable)]`.

Native needs nothing new, because guests already open the live socket through `leptoskit_plugin_websocket`.

I considered a separate `use_event_source::<T>(url)` hook and rejected it. It would be a second streaming API, and on native it would need streaming response bodies in `leptoskit_plugin_http`, which buffers the whole body today (`crates/leptoskit_plugin_http/src/lib.rs:150,291-296`).

### G4. Scroll into view (C, S-M)

The problem: only a `ScrollView`'s numeric offset can be set. Six flows scroll a particular element into view, two of them to the centre, and one "only if it's out of view, scrolling only the drawer".

```rust
let reveal = Reveal::new();                       // one-shot, like FocusRequest
Button("Sign in")
    .reveal_on(&reveal, ScrollAlign::Center)      // Start, Center, End, Nearest
    .focus_on(&to_verify);
reveal.request();
```

- **Web:** `scrollIntoView({ block, behavior })` in a microtask, the same as `scroll_after_render`. `behavior: "smooth"` is used only when `prefers-reduced-motion` is off, which matches what the scripts do by hand. `Nearest` does nothing when the element is already visible.
- **Native:** the surface already knows every frame from layout. It finds the nearest `ScrollView` ancestor and sets the offset through the `scroll_offset` path that the generated macOS, iOS and Android apps already apply. The backend crates (GTK, Windows) still need `scroll_offset` itself (`docs/src/readme.md:130`).

### G5. Roving focus composites in `leptoskit_aria` (B, M)

The problem: tab lists, radio cards and menus should be one Tab stop, with Arrow, Home and End keys moving between items. `on_key` now exists on the web and in the terminal, so the reason given in `leptoskit_aria/src/lib.rs:59-62` and `docs/src/guide/16-accessible-components.md:61` is stale.

```rust
let tabs = Tabs::new("ask", 3, selected.get()).orientation(Orientation::Horizontal);
tabs.list(move |index| selected.set(index))     // attaches on_key: Arrows, Home, End move and select
    .child(tabs.tab(0, "Rust"))
    .child(tabs.tab(1, "SQL"))
    .child(tabs.tab(2, "HTTP"))

RadioGroup::new("persona", ["ada", "grace"], chosen.get())
    .option(0, ada_card)                          // any content; role=radio, aria-checked, roving
    .option(1, grace_card)
    .on_change(move |index| chosen.set(Some(index)))

Menu::new("account", open.get())                  // built on G1 Popover
    .button(avatar)                               // aria-haspopup, aria-expanded, aria-controls
    .item("Open the admin", open_admin)
    .item("Sign out", sign_out)                   // Up, Down, Home, End; Escape and Tab close; focus returns
```

Tabs use `tab_index(if selected {0} else {-1})` with G2 requests. A radio group selects as focus moves. A menu moves focus without selecting.

Native gets the same behaviour once G7b delivers `on_key` there. Until then, natively every item stays tabbable, as now.

### G6. Platform clock hook (B, S)

```rust
let now = leptoskit_use::use_clock(Duration::from_secs(1));   // Signal<Duration>
let left = move || seconds_to_reset(server_now_ms + now.get().as_millis() as i64);
```

This is a loop over `leptoskit_query::try_sleep`, which uses `setTimeout` in the browser and the timer plugin in native guests. The loop is cancelled on owner cleanup, and it returns the `Clock` type that `use_timeout_fn`, `use_interval_fn` and `signal_throttled` already accept. Native needs no new backend work.

### G7. Keyboard: ⌘ and app shortcuts (a: B, S; b: C, L)

**(a)** Add `meta: bool` to `KeyPress`, and `KeyPress::is_primary()` (⌘ on Apple, Ctrl elsewhere). Let the web dispatch ⌘ keys to handlers, since `web/mod.rs:1060` skips them all today. Keep the browser's own shortcuts for ⌘ keys no handler takes. This unblocks ⌘+Enter (DM-54) on the web.

**(b)** App-level shortcuts that fire with nothing focused:

```rust
use_shortcut(Shortcut::primary("k"), move || search_focus.request());
use_shortcut(Shortcut::key("w").unless_typing(), toggle_drawer);
use_shortcut(Shortcut::key("W").shift(), toggle_workings_mode);
```

- **Web:** a `window` `keydown` listener, because the render root never sees keys aimed at `<body>`.
- **AppKit:** a local `NSEvent` key-down monitor in the host, or menu key equivalents through `leptoskit_plugin_menu`.
- **UIKit:** `keyCommands`/`UIKeyCommand` on the root view controller, for hardware keyboards.
- **Android:** `Activity.dispatchKeyEvent`/`onKeyShortcut`.
- **GTK:** a `gtk::ShortcutController` with `ShortcutScope::Global` on the window.
- **Windows:** an accelerator table (`TranslateAccelerator`), or `WM_KEYDOWN` in the message loop.

The same PR should deliver element-level `on_key` natively: AppKit `keyDown:` and the text field's `doCommandBySelector`, UIKit `pressesBegan`, Android `OnKeyListener`, GTK `EventControllerKey`, and Win32 subclassing.

### G8. Typed refusals in the rpc client (B, S-M)

The problem: the demo's copy depends on why something was refused: rate limited (with how long to wait), the day is full, signed out versus a new day, or the notebook is full. The client only sees `FnError::{Network, Invalid, Server, Decode, Other}(String)`.

```rust
// server
return Err(rpc::Error::rate_limited(Duration::from_secs(retry)));
return Err(rpc::Error::refused("notes_full", "Your notebook is full"));   // 409 by default
// client
match error {
    FnError::Refused { code, retry_after, .. } if code == "rate_limited" => …,
    …
}
```

Add `Error::RateLimited { retry_after }` (429, with `Retry-After`) and `Error::Refused { code, message, status }`, and carry them into a new `FnError::Refused { status, code, message, retry_after }`. This mirrors `leptoskit_auth::ErrorResponse` (`error`, `message`, `retry_after`, `api.rs:382`), so sign-in and app operations read the same way. Natively nothing is needed; it is transport-agnostic.

Workaround available today: return the refusals as `Ok` enum variants. It works, but every app reinvents it.

### G9. Text input hints (B, S-M)

```rust
TextInput()
    .keyboard(Keyboard::Email)          // Text (default), Email, Number, Decimal, Phone, Url, Search
    .read_only(true)                    // implies aria-readonly
    .max_length(9)
    .capitalize(Capitalize::Characters) // None, Words, Sentences, Characters
    .autocorrect(false)                 // also turns off spelling marks
    .submit_label(SubmitLabel::Go)      // Done, Go, Next, Search, Send
```

| Prop                 | Web                                                      | AppKit                                                               | UIKit                                          | Android                                                      | GTK                                | Windows          |
| -------------------- | -------------------------------------------------------- | -------------------------------------------------------------------- | ---------------------------------------------- | ------------------------------------------------------------ | ---------------------------------- | ---------------- |
| `keyboard`           | `type=email/tel/url/search`, `inputmode=numeric/decimal` | none (content type for autofill only)                                | `keyboardType`                                 | `inputType` variation                                        | `set_input_purpose`                | none             |
| `read_only`          | `readonly`                                               | `isEditable = false`, `isSelectable = true`                          | delegate refuses edits; field still selectable | `isFocusable`+`isCursorVisible` off, or `keyListener = null` | `set_editable(false)`              | `EM_SETREADONLY` |
| `max_length`         | `maxlength`                                              | delegate `controlTextDidChange` truncates                            | `shouldChangeCharactersIn`                     | `InputFilter.LengthFilter`                                   | `set_max_length`                   | `EM_LIMITTEXT`   |
| `capitalize`         | `autocapitalize`                                         | none                                                                 | `autocapitalizationType`                       | `TYPE_TEXT_FLAG_CAP_*`                                       | `InputHints::UPPERCASE_CHARS` etc. | none             |
| `autocorrect(false)` | `autocorrect=off spellcheck=false`                       | field editor: automatic correction and continuous spell checking off | `autocorrectionType`, `spellCheckingType`      | `TYPE_TEXT_FLAG_NO_SUGGESTIONS`                              | `InputHints::NO_SPELLCHECK`        | none             |
| `submit_label`       | `enterkeyhint`                                           | none                                                                 | `returnKeyType`                                | `imeOptions` (`IME_ACTION_GO` …)                             | none                               | none             |

`name` is left out on purpose. It has no native meaning, and `autocomplete` already guides password managers (`element.rs:1572-1581`). The props also apply to `TextArea` where they make sense.

### G10. Document title, and optionally fragments (B, S)

- **Title:** add `leptoskit_router::set_title(…)`, or a per-route `title`. It maps to `document.title` on the web, `NSWindow.title` on macOS, `set_title` in GTK, `SetWindowTextW` on Windows, and nothing on mobile. leptos_meta's `<Title>` covers the web today.
- **Fragments:** `RouteUrl::fragment()` and `Router::set_fragment(Option<&str>)` using `replaceState`. **I recommend not doing this now.** The admin owns its URLs, so moving `#workings=plan` to `?workings=plan` makes AD-06 work today.

### G11. Pre-hydration fallbacks (B, optional, S)

- Add a `noscript:` variant to the class compiler, emitted as `@media (scripting: none)`, to replace the `data-js` inline script (`pages/mod.rs:181`). It supports the "Writing notes needs scripts" text (`notes.rs:305-307`). Native ignores the variant (it always has scripts).
- If the owner wants the front door's fallback in leptoskit markup rather than a raw HTML string, add a minimal `Form().action("/demo/pass").method(Post)` with `Button().submit_value("persona", "ada")`. This renders a real `<form>` that hydration takes over. It is web-only by nature.

### G12. `TextArea` (C, M)

```rust
TextArea()
    .id("note-text")
    .value(draft.get())
    .on_change(move |text| draft.set(text.to_owned()))
    .read_only(saving.get())
    .autocomplete("off")
    .on_key(move |key| if key.key == "Enter" && key.is_primary() { save(); EventOutcome::Handled } else { EventOutcome::Unhandled })
    .class("min-h-[120px] w-full …")
```

**Recommendation: a new `NativeKind::TextArea` and a `TextArea()` component, not `TextInput().multiline(true)`.**

- Four of the six targets use a different widget class: `<textarea>`, `NSTextView` in an `NSScrollView`, `UITextView` and `gtk::TextView`. A kind is fixed when the widget is created, so flipping a prop would make every backend tear the widget down and rebuild it.
- Enter means something different: it adds a new line rather than submitting. The web already raises `Submit` only for `HtmlInputElement` (`web/mod.rs:1079-1093`).
- Layout differs: the height grows with the content.

`aria-multiline` is implied. Validation should reject `aria_multiline` on a `TextInput`, because it misreports the field today (`notes.rs:316`).

| Platform | Work                                                                                                                                              |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Web      | `html::textarea()`; teach `controls::text_event` and `restore_input` the `HtmlTextAreaElement` (`web/controls.rs:248-252` expects an input today) |
| AppKit   | `NSScrollView` + `NSTextView`, `textDidChange` → `Change`; placeholder drawn by the renderer (`NSTextView` has none)                              |
| UIKit    | `UITextView` + `textViewDidChange`; placeholder label overlay                                                                                     |
| Android  | `EditText` with `TYPE_TEXT_FLAG_MULTI_LINE`, `gravity = TOP`, `isSingleLine = false`                                                              |
| GTK      | `gtk::TextView` in a `gtk::ScrolledWindow`; the buffer's `changed` signal                                                                         |
| Windows  | `EDIT` with `ES_MULTILINE`, `ES_AUTOVSCROLL`, `ES_WANTRETURN` and `WS_VSCROLL`; `EN_CHANGE`                                                       |
| Surface  | measure the field from its line count and line height, with `min-h-*` respected                                                                   |
| Terminal | a multi-line editor in `crates/leptoskit_tui/src/input.rs`                                                                                        |

### G13. Labels focus their fields (B, S)

No new API is needed. During `web::render`, a `Text` whose `id` is the only `aria_labelled_by` target of a text field, checkbox, switch, radio or slider renders as `<label for="field-id">`, so a click focuses or toggles the field. `leptoskit_aria::Field` already produces exactly this shape (`crates/leptoskit_aria/src/field.rs`). Validation already collects every ID (`element.rs:214-217`), so the pre-pass is cheap.

Natively, Android calls `setLabelFor`. AppKit, GTK and Windows already get the relationship for accessibility from `aria_labelled_by`. Clicking a label to focus its field is not a convention on Apple platforms, so they have nothing to do.

### G14. Links that open a new tab (B, S)

```rust
Pressable().href(admin_url).target(LinkTarget::NewTab).child(Text("Open the admin ↗"))
```

On the web this renders `target="_blank" rel="noopener noreferrer"` and adds "Opens in a new tab" as an `aria-description`. Native ignores `href` today (`element.rs:1189-1195`). Native link opening should go through `leptoskit_plugin_linking` (its web module already opens with `noopener`, `crates/leptoskit_plugin_linking/src/lib.rs:234-245`); that is a prerequisite worth doing in the same PR.

### G15. Download links (B; S on the web, M natively)

```rust
Pressable().href(export_url).download("person-7.json")
```

The web renders `<a href download>`, and the server keeps sending `Content-Disposition: attachment`. Natively, the app fetches with the HTTP plugin, then calls `save_file` from the dialog plugin on desktop (`crates/leptoskit_plugin_dialog/src/desktop.rs:152`), or opens the share sheet with `FileSource::Bytes` on mobile. In the admin, the confirmation sheet's "Export" button becomes this link, so a confirmed press downloads with no script.

### G16. `prompt=login` on provider sign-in (B, S)

Add `prompt: Option<Prompt>` to `StartProvider` (`api.rs:554-561`), with the values `Login` and `SelectAccount`, and pass it to the authorization URL. `oauth.rs` already appends `prompt=select_account` for a quirk (`crates/leptoskit_auth/src/oauth.rs:142,1404`).

### G17. Announcer (B, optional, S)

`leptoskit_aria::Announcer` would provide a polite region and an assertive one, with `announce(text)` adding an invisible marker so identical text is read again. This generalises the toaster's private trick (`components/toast.rs:910-927`).

Natively, it could use the platform's announcement APIs:

- **AppKit:** `NSAccessibility.post(.announcementRequested)`
- **UIKit:** `UIAccessibility.post(.announcement)`
- **Android:** a live region (`announceForAccessibility` is deprecated in API 33)
- **GTK:** `announce` needs GTK 4.14, so it stays unannounced at GTK 4.6
- **Windows:** `UiaRaiseNotificationEvent`

---

## 6. What should not move into Wasm

The bar is high; these items qualify:

1. **The "opening today's demo" page** (`examples/demo_service/src/pages/opening.rs:3,75`) stays a static page with a `<meta http-equiv="refresh">`. It is served while a generation builds, when that generation's client bundle may not exist yet.
2. **First-paint and no-script paths:**
   - **`data-js` flag:** the one-line script (`pages/mod.rs:181`) must take effect before first paint, and Wasm arrives after it. Replace it with CSS `@media (scripting: none)` (G11); don't port it.
   - **Mint form:** keep the `<noscript>` form (`pages/front.rs:467-478`) posting form-encoded data to `/demo/pass`, so the front door works before or without Wasm.
3. **The theme's first paint** stays on the server, which writes `data-lk-appearance` on `<html>` from the stored preference (`crates/leptoskit_admin/src/shell.rs:777-779`). `appearance.rs:24-27` says the same. Wasm only changes it later.
4. **The person export** stays a server attachment. Wasm only triggers the download (G15). It should never pull a person's data into Wasm memory to build a Blob, as `admin.mjs:993-1004` does with JS.
5. **Stream lifetimes stay on the server.** The permits, keep-alives and bounded windows (`examples/demo_service/src/dispatch.rs:170-217`, `examples/demo_service/src/admin.rs:9-20`) are server behaviour. Only the subscription moves.

Two constraints for the port, not reasons to keep JS:

- **Clipboard:** writes must happen synchronously inside the press handler, with no `await` first, or the browser drops the user activation.
- **Countdown:** it must keep measuring from the server's clock (`demo.js:30`).

Other hand-written JS outside this audit, for the owner to schedule:

| File                                                        | Lines    | What it is                  |
| ----------------------------------------------------------- | -------- | --------------------------- |
| `crates/leptoskit_deploy_console/src/control/dashboard.mjs` | 134      | the console UI              |
| `crates/leptoskit_cli/src/devtools/page.html`               | 195      | the DevTools page           |
| `crates/leptoskit_docs/assets/site.js`, `theme.js`          | 440 + 30 | the docs site               |
| `crates/leptoskit_app/web/*.mjs`, `bootstrap.js`            | 422      | landing page and playground |
| `docs/src/theme/play.js`, `play-frame.js`                   | 289      | playable examples           |

The service worker (`crates/leptoskit_notifications/src/browser/receiver.js`) and the APM beacon (`crates/leptoskit_apm/src/browser.mjs`) are not UI.

---

## 7. Suggested PR sequence

Each PR is one coherent leptoskit feature with web, terminal and native parity, or an explicit native follow-up in the same stack. The order unblocks the demo first, since it is smaller, public and the showcase; the admin's extra needs come after. Within that, PRs that establish shared plumbing come first.

| #      | PR                                                                                                                  | Gaps     | Depends on  | Size   |
| ------ | ------------------------------------------------------------------------------------------------------------------- | -------- | ----------- | ------ |
| 1      | `feat(native): text input hints` (keyboard, `read_only`, `max_length`, capitalisation, autocorrect, submit label)   | G9       | —           | S-M    |
| 2      | `feat(native): TextArea`                                                                                            | G12      | 1           | M      |
| 3      | `feat(native): labels focus the field they name`                                                                    | G13      | —           | S      |
| 4      | `feat(native): one-shot focus requests`; native backends honour focus (can be stacked per backend)                  | G2       | —           | M      |
| 5      | `feat(aria): roving Tabs, RadioGroup, and Menu`; update the stale docs                                              | G5       | 4           | M      |
| 6      | `feat(native): Modal and Popover overlays` (web, terminal and AppKit, then UIKit and Android, then GTK and Windows) | G1       | 4           | L      |
| 7      | `feat(native): scroll an element into view`                                                                         | G4       | —           | S-M    |
| 8      | `feat(use): platform clock`                                                                                         | G6       | —           | S      |
| 9      | `feat(rpc): typed refusals`                                                                                         | G8       | —           | S-M    |
| 10     | `feat(rpc): streams without a database`                                                                             | G3       | —           | M      |
| 11     | `feat(native): ⌘ and the primary modifier in key presses`                                                           | G7a      | —           | S      |
| 12     | `feat(native): link targets and downloads` (plus native link opening through the linking plugin)                    | G14, G15 | —           | S-M    |
| **13** | **Demo → hydrated leptoskit** (3–4 PRs, section 8)                                                                  | —        | 1–12        | L      |
| 14     | `feat(native): app shortcuts and native key delivery`                                                               | G7b      | 11          | L      |
| 15     | `feat(auth): prompt on provider sign-in`                                                                            | G16      | —           | S      |
| 16     | `feat(router): document title`                                                                                      | G10      | —           | S      |
| **17** | **Admin → hydrated leptoskit** (5–7 PRs, section 8)                                                                 | —        | 4–12, 14–16 | XL     |
| 18     | optional: `feat(aria): announcer`, `feat(native): noscript variant and form post`                                   | G17, G11 | —           | S each |

Each feature PR carries a changeset, updates the platform pages and the "Where things stand" list in `docs/src/readme.md`, and adds a gallery or probe screen so CI exercises it on every backend.

---

## 8. Rough size of each conversion, once the gaps are closed

### Demo: L, 3–4 PRs

Starting point: four pages that are already leptoskit `view!` markup (`front.rs` 478, `sign_in.rs` 655, `notes.rs` 493, `chrome.rs` 439 lines).

Work:

1. **Hydrated client.** Move the pages into a crate that builds for both the server and `wasm32` (`demo_service` is `cfg(unix)`-only, `examples/demo_service/Cargo.toml`). Serve it with leptos SSR and `HydrationScripts` instead of the hand-built document in `pages/mod.rs:161-195`. Set a `[bundle] budget_kib`, because this is the public front door.
2. **Operations.** Turn `/demo/pass`, `/demo/inbox` and `/demo/notes` into rpc operations, and keep the form-encoded `/demo/pass` POST for the no-script form. Turn `/demo/generation` into a stream on a live mount without a database. Sign-in uses `leptoskit_auth::client` directly.
3. **State.** Replace the `html[data-state|data-band|data-route|data-reset]` CSS state machine with signals.

Deltas:

- Deletes `demo.js` (1,513 lines) and about 350–450 lines of the 741-line `demo.css`.
- Adds about 1,500–2,200 lines of Rust.
- `examples/demo_service/tests/visitor_pages.mjs` (1,818 lines) stays the acceptance suite if test IDs and copy are kept.

### Admin: XL, 5–7 PRs

Starting point: about 16,000 lines of server-side leptoskit page code across 19 GET pages and 8 JSON endpoints (`crates/leptoskit_admin/src/server.rs:60-78`, `analytics/routes.rs:57-79`).

**Recommendation: hydrate page by page (MPA) first.** Each page server-renders and hydrates its own state; rail links stay full navigations. Only the analytics builder (AD-24) refetches through rpc queries in place. This avoids moving every page's data loading to the client up front. It still needs `serde` on the view models the pages hydrate from, and a decision on packaging. `leptoskit_admin` is a library behind the deployable `examples/admin_demo` (no `leptoskit.toml` today), and `demo_service` serves the site and the admin from one process. Either build one client crate with both roots, split with `#[lazy]`, or build two clients. This is a build question, not a framework gap.

PRs:

1. Client crate and shell: theme, avatar menu, search, shortcuts, drawers.
2. Infrastructure: actions and operation progress.
3. Analytics builder: in-place query swaps and changed-line washing.
4. Live page: pulse and tail.
5. People, insights and dashboards dialogs.
6. Delete `admin.mjs` and the state rules in `shell_css`.

Deltas:

- Deletes `admin.mjs` (1,403 lines) and about 80 lines of state CSS in `shell.rs`.
- Adds about 2,500–4,000 lines of Rust.
- `crates/leptoskit_admin/tests/script.mjs` (593 lines) stays the acceptance suite if test IDs are kept.

### Risks to watch in both

- **Re-rendering cost.** Each `render` re-emits its stylesheet. Rebuilding a whole screen once a second (the countdown and the pulse) is wasteful, so bind fast-changing values with `text_from`, `For` and `Show` regions (`reactive.rs:1-16`).
- **Validation panics.** `web::render` panics on an invalid tree (`web/mod.rs:106-108`), so hydrated pages need the same validation tests the SSR pages have.
- **Clock-dependent text.** Anything derived from time (the countdown) must render identically on the server and on the first client pass, so compute it from the server's `now_ms`, not the browser clock.
- **Islands.** The islands render mode could cut the demo's bundle, but I did not verify that leptoskit `web::render` works inside `#[island]`.
