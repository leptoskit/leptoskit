---
"release": patch
---

# Retry stalled TypeScript live connections

Generated TypeScript RPC clients retry a connection if the WebSocket upgrade or the server's welcome does not finish within ten seconds. Subscriptions and pending writes survive the retry, and late callbacks from the abandoned socket cannot replace the new connection. Set `live.connectTimeoutMs` to allow a different connection deadline.
