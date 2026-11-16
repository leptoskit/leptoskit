---
"leptoskit_notifications": patch
---

Push signing no longer pulls in the `rsa` crate. The FCM service account's RS256 OAuth assertion is built and signed with ring's constant-time RSA directly — the same primitives APNs's ES256 provider token already used — so `jsonwebtoken` and its RustCrypto stack leave the dependency tree. Startup refusals, claims, caching, and the wire contract are unchanged.
