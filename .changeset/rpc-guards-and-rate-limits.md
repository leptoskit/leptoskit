---
"leptoskit_rpc": minor
"leptoskit_rpc_macros": minor
"leptoskit_cli": minor
---

# Rate limits, timeouts, and guards for public operations

Every query, mutation, action, and stream a client can call can now be limited before its body runs, over HTTP and over a live connection alike.

```rust
use leptoskit::rpc::server::{Rate, RateLimit, Server};

let rpc = Server::new(db)
	.identify(auth)
	.client_ip_header("fly-client-ip")
	.rate_limit(RateLimit::new().anonymous(Rate::per_minute(60)))
	.guard(SignedInOnly);
```

- `Server::rate_limit(RateLimit)` counts each caller: by address when nobody is signed in (IPv6 by its /64), and by `Identity::caller` when someone is, with `RateLimit::key` to count every session of one person together. The defaults are 120 calls a minute per address and 600 per signed-in caller, refilled evenly (GCRA). Services and the app calling them aren't counted unless `RateLimit::internal` says so. A refused call is `429` with `Retry-After`, which clients already read as `FnError::Refused` with the code `rate_limited`.
- Counts live in a bounded `MemoryStore` by default. Implement `RateStore` to share them between servers, for example over Redis.
- `#[rpc::query(rate_limit = 10 / 1m, timeout = 5s, max_body = 64KiB)]`, and the same on `#[rpc::mutation]` and `#[rpc::action]`, set one operation's limits. An operation's own `rate_limit` holds even when the server sets no `RateLimit`. They are on `Registration::limits` as `OperationLimits`.
- A call past its timeout is answered `504` with the code `timed_out` (`rpc::server::TIMED_OUT`); arguments over the size limit are answered `413` with `too_large` (`rpc::server::TOO_LARGE`) instead of a `400`. `Server::timeout` and `Server::max_body` set the defaults; the body limit is still 2 MiB unless changed.
- `Server::guard(impl Guard)` runs an app's own check before every call. A `Guard` sees a `Call`: the operation's name and kind, the identity, the client's address, and whether it came over HTTP or a live connection. `Live` and `LiveStreams` take the same `guard`, `rate_limit`, and `client_ip_header`, and `Server::live()` passes its server's on.
- The HTTP routes now identify the caller and run the guards before reading the request body, so a refused caller costs no more than their headers. Calls the server makes in process while it renders a page are not checked again.

Rate limiting is opt-in for existing apps. New database and sign-in projects from `leptoskit new` start with `Server::new(db).rate_limit(RateLimit::new())` in the backend's `operations`.

`Registration` has a new `limits` field: code that builds one by hand adds `limits: OperationLimits::NONE`.
