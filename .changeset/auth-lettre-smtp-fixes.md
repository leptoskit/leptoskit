---
"release": patch
---

# Sign-in email picks up six SMTP security fixes

`leptoskit_auth`'s `smtp` feature sends sign-in email through `lettre`, which published fixes for six advisories (RUSTSEC-2026-0560 to RUSTSEC-2026-0565): a man-in-the-middle forging replies on a `STARTTLS` connection, command injection through a bare line break in a message, a message for an address with a quoted local part going to a different address, and three ways a malicious server or a crafted display name could crash or exhaust the sender. The locked version moves from 0.11.23 to 0.11.24. Nothing changes in how you configure or call it.

If your app has its own lockfile, run `cargo update -p lettre` to take the fixed release.
