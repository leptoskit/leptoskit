# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

#### A fallible wait for hosts without a timer

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #609](https://github.com/leptoskit/leptoskit/pull/609)

`leptoskit_query::try_sleep` reports `SleepError::Unavailable` or `SleepError::Rejected` instead of pretending a wait elapsed. Generated sign-in screens use it for a rate-limit countdown, so on a host with no usable timer the button becomes a manual retry, the server's "try again in …" message stays visible, and the screen never claims time that did not pass. The server remains the authority on when the window opens.

This is the compatibility surface for the native guest timer work. When that timer lands, its host-backed `try_sleep` implementation replaces the fallback here without changing the screen's behavior.

#### Screen state and queries survive a rebuild

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #736](https://github.com/leptoskit/leptoskit/pull/736) · _Closed issues:_ [#721](https://github.com/leptoskit/leptoskit/issues/721)

leptoskit builds a screen again whenever a signal it read changes. Two things broke under that: a `RwSignal::new(..)` inside a screen ran again with every rebuild, so a chosen filter or a typed field reset as soon as the screen redrew, and a `use_query` in a screen refetched in a loop, because the rebuild its data caused mounted the query again while the data already counted as stale.

- `leptoskit_native`: `remember(|| initial)` returns the same `RwSignal` on every build of the screen, told apart by its place in the source and by order. `remember_keyed(&key, || initial)` names the value by a key instead, for rows that move. A value is released once a whole build no longer reaches it, such as after navigating to another route. `guest::mount` keeps the values for native apps. In the browser, `web::screen(move || ..)` replaces `move || web::render(..)` and does the same. `ScreenState` is the store underneath, for hosts of their own. Both functions are in `leptoskit::prelude`, and the module is `leptoskit::remember`.
- `leptoskit_query`: a query mounted within two seconds of this client's last fetch of it takes that fetch's data or error instead of fetching again, like SWR's deduping interval. The window covers `RefetchOn::Always` and `retry_on_mount`, and not invalidation, `refetch()`, focus, reconnect, or intervals. Data from `set_query_data`, initial data, or hydration doesn't count as a fetch. Set it with `with_mount_dedupe` on `QueryDefaults`, `QueryOptions`, or `QueryObserverOptions`; `Duration::ZERO` gives TanStack's behaviour. `QueryOptions`, `QueryDefaults`, and `ResolvedQueryOptions` gain a `mount_dedupe` field, so code that builds them with struct literals needs it too.

An app with its own keyed store and a long stale time to work around this can switch to `remember` and drop both.

#### Server-rendered queries hydrate without a mismatch

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #756](https://github.com/leptoskit/leptoskit/pull/756) · _Closed issues:_ [#722](https://github.com/leptoskit/leptoskit/issues/722)

A page that rendered a query on the server now hydrates as the same page in the browser. Before, only a query's data was carried, so a query that failed on the server started from "loading" in the browser, leptos reported a hydration mismatch, and the page stopped updating.

- `QueryScope::with_ssr_errors` (and `with_error_codec`) streams a server render's error alongside the data, so the browser shows the same error first. RPC queries and paginated queries opt in, and `FnError` is now `Serialize` and `Deserialize`.
- A query seeded from the server doesn't fetch while the page hydrates. Its mount refetch — stale data in the background, or a retry after an error — starts once hydration has finished, so the first render matches the server's HTML.
- On the server, reading a query that is still loading inside `<Suspense>` holds the boundary until it settles, as a leptos resource does, so the server renders the outcome it streams.
- Streamed payloads escape `<`, `>`, `&`, and U+2028/U+2029, so data and errors can't close the `<script>` they're embedded in.

Apps using `render = "ssr"` should read queries inside `<Suspense>`; outside a boundary the server renders whatever the query holds at that moment.

New apps put their web adapter inside that boundary, including the database and sign-in scaffolds. Existing portable apps should wrap the closure that calls `leptoskit::web::render` in the same way.

Generated browser entrypoints configure the RPC and auth clients inside the mount callback, after leptos has set up its executor and owner, so the live socket starts on the first page load. A query observer reads `enabled` without subscribing while it binds or runs its refetch interval, so an `enabled` closure that reads another query no longer logs a reactive-tracking warning on every page.

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_query: async state in the TanStack style

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_query` is TanStack Query for leptos: screens that load data get caching, deduplication, retries, background refetching, infinite queries, and optimistic mutations, all exposed as fine-grained signals so a view re-renders only when the value it reads changes.

Declare a `QueryScope` or `MutationScope` and read it with `use_query` or `use_mutation`. Queries refetch on focus and reconnect, pause offline, stay in sync across browser tabs or native windows through `QueryClient::broadcast`, and can be fetched during server rendering with `with_ssr`. The default `browser` feature wires up the browser's timers and events; native guests enable `guest` instead, and the `db` feature refetches the queries a `leptoskit_db` live change touches. `leptoskit_rpc` builds its client calls on it.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
