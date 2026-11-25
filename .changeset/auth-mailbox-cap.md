---
"leptoskit_auth": patch
---

# The development mailbox keeps the newest 500 emails

`Mailbox` kept every email it was given for as long as the process ran. An app whose callers can trigger email, as a public demo's sign-in codes and invitations do, grew without end. It now keeps the newest 500, `Mailbox::CAP`, and drops the oldest past that, so what it holds and the page that shows it stay small.
