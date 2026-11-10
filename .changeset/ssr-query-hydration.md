---
"leptoskit_query": minor
"leptoskit_rpc": minor
"leptoskit_cli": patch
---

# Server-rendered queries hydrate without a mismatch

A page that rendered a query on the server now hydrates as the same page in the browser. Before, only a query's data was carried, so a query that failed on the server started from "loading" in the browser, leptos reported a hydration mismatch, and the page stopped updating.

- `QueryScope::with_ssr_errors` (and `with_error_codec`) streams a server render's error alongside the data, so the browser shows the same error first. RPC queries and paginated queries opt in, and `FnError` is now `Serialize` and `Deserialize`.
- A query seeded from the server doesn't fetch while the page hydrates. Its mount refetch — stale data in the background, or a retry after an error — starts once hydration has finished, so the first render matches the server's HTML.
- On the server, reading a query that is still loading inside `<Suspense>` holds the boundary until it settles, as a leptos resource does, so the server renders the outcome it streams.
- Streamed payloads escape `<`, `>`, `&`, and U+2028/U+2029, so data and errors can't close the `<script>` they're embedded in.

Apps using `render = "ssr"` should read queries inside `<Suspense>`; outside a boundary the server renders whatever the query holds at that moment.

New apps put their web adapter inside that boundary, including the database and sign-in scaffolds. Existing portable apps should wrap the closure that calls `leptoskit::web::render` in the same way.

Generated browser entrypoints configure the RPC and auth clients inside the mount callback, after leptos has set up its executor and owner, so the live socket starts on the first page load. A query observer reads `enabled` without subscribing while it binds or runs its refetch interval, so an `enabled` closure that reads another query no longer logs a reactive-tracking warning on every page.
