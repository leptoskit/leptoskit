---
"leptoskit_rpc": minor
---

# `AppCaller`: call a service as the app from another program

`AppCaller::new(service, url, token)` calls one service's operations as the app, the same way `call_as_app` does, but it uses the address and token you hand it instead of reading them from the environment. It's for a separate program that acts for the app, such as its admin. `.call(op)` refuses an operation that belongs to a different service. Keep it on the server, like the token it holds.
