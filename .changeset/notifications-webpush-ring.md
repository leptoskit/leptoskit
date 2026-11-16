---
"leptoskit_notifications": patch
---

Web Push now encrypts (RFC 8291 `aes128gcm`) and signs VAPID (RFC 8292 ES256) directly on ring instead of through the `web-push` crate, whose `jwt-simple`/`ece` dependency chain pulled the `rsa` and `crypto-bigint` advisories. Each message draws a fresh ephemeral P-256 key and salt, the RFC 8291 Appendix A vector is asserted byte for byte in the tests, and the wire is unchanged: same headers, same envelope, same VAPID header shape.
