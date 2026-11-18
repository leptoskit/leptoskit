---
"leptoskit_auth": patch
---

# SMTP security fixes

`SmtpMailer` now requires `lettre` 0.11.24, which fixes six advisories published against 0.11.23 (RUSTSEC-2026-0560 to RUSTSEC-2026-0565): a man-in-the-middle could inject replies during STARTTLS, message content with bare line endings could inject SMTP commands, an address with a quoted local part could be sent to a different address, and a malicious server or a display name with a line break could crash or exhaust the sender. Apps that send sign-in emails over SMTP should update.
