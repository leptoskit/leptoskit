---
"leptoskit_query": minor
"leptoskit_rpc": minor
---

# Server-rendered queries hydrate without a mismatch

A page that rendered a query on the server now hydrates as the same page in the browser. Before, only a query's data was carried, so a query that failed on the server started from "loading" in the browser, leptos reported a hydration mismatch, and the page stopped updating.

- `QueryScope::with_ssr_errors` (and `with_error_codec`) streams a server render's error alongside the data, so the browser shows the same error first. RPC queries and paginated queries opt in, and `FnError` is now `Serialize` and `Deserialize`.
- A query seeded from the server doesn't fetch while the page hydrates. Its mount refetch — stale data in the background, or a retry after an error — starts once hydration has finished, so the first render matches the server's HTML.
- On the server, reading a query that is still loading inside `<Suspense>` holds the boundary until it settles, as a leptos resource does, so the server renders the outcome it streams.
- Streamed payloads escape `<`, `>`, `&`, and U+2028/U+2029, so data and errors can't close the `<script>` they're embedded in.

Apps using `render = "ssr"` should read queries inside `<Suspense>`; outside a boundary the server renders whatever the query holds at that moment.
