---
"leptoskit_rpc": patch
"leptoskit_rpc_macros": patch
---

A stream body that omits its context and reads it with `rpc::cx()` now sees the query context the server passed in, while it is constructed and while its items are polled. Streams ran outside the query task, so `rpc::cx()` answered "outside a query" and the stream failed; the whole body now runs inside the context's task — constructing the stream and driving its items — the same as queries and mutations, so a lazily-polled stream keeps the context too.
