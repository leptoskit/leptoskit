---
"release": patch
---

# Huddle counts its callers and limits invitations

Huddle's API now counts each caller, over HTTP and the live socket alike: 120 calls a minute from an address nobody is signed in from, and 600 a minute for someone signed in. Past that, a call is answered `429` with how long to wait. Behind a proxy, callers are counted by the address in the header sign-in already reads (`HUDDLE_CLIENT_IP_HEADER`), so they don't all share the proxy's allowance. A caller may also send ten invitations an hour, and the next is answered `429` until one is free: each leaves a row, a job, and an email in the demo mailbox, which keeps the newest 500.
