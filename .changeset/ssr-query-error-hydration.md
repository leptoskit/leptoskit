---
"leptoskit_query": patch
"leptoskit_rpc": patch
---

# Server-rendered query errors hydrate

A screen that renders from a query's error during server rendering drew that error into the HTML, then shipped `null` in the hydration payload. The browser started from loading, the two trees did not match, and hydration stopped, so the page never updated.

Scopes can now opt in with `.with_ssr_error()` (next to `.with_ssr()`). A settled error is streamed with the HTML and applied before the first browser render. `use_rpc_query` and paginated RPC queries do this; `FnError` serializes so the error the server showed is the error the browser starts from. The refetch still happens, after that render, so a later success can replace it.
