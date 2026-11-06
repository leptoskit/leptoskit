---
"release": patch
---

# Local deployment services ignore proxy settings

`lk console`, `lk deploy`, `lk infra plan`/`apply`, and the hosted worker now connect straight to a deployment service on `localhost`, `127.0.0.1`, or `[::1]` even when `ALL_PROXY`, `HTTPS_PROXY`, or `HTTP_PROXY` is set. Before, every request went to the proxy, which can't reach your own machine, and failed with "deployment service request failed". Remote HTTPS services still use your proxy.
