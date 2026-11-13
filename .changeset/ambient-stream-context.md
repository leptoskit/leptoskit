---
"leptoskit_rpc": patch
"leptoskit_rpc_macros": patch
---

A stream body that omits its context and reads it with `rpc::cx()` now sees the query context the server passed in. Streams ran outside the query task, so `rpc::cx()` answered "outside a query" and the stream failed; the body now runs inside the context's task, the same as queries and mutations.
