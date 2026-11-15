---
"leptoskit_admin": minor
---

# The public demo's service runtime

`examples/demo_service` is the public demo's process: one image per UTC day, a daily generation published with a durable marker under kernel file locks, and a same-PID re-exec at midnight. It serves the site, the admin, and a private loopback (health checks and the token-authenticated analytics prefix), refuses every other `Host` with 421, and caps the `/demo/generation` event stream at 1,000 connections. Successful operation event streams end after 15 seconds so an overlapping candidate reconnects into the durable terminal state. Visitor onboarding, visual design, and provider deployment are later tasks; operator rules live in `docs/agents/public-demo.md`.
