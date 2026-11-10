---
"release": minor
---

# Let operator apps sign in without PKCE

`leptoskit::idp` still requires PKCE by default, but an operator can now let one app sign in without it, for an OpenID Connect library that can't send it, or the OpenID Foundation's certification tests.

- **`leptoskit auth idp client add --pkce-optional`** (`NewClientSpec::require_pkce = false`) is accepted only for an app with a secret or a key. Public apps always use PKCE, and apps from the developer portal or dynamic registration can't ask for the opt-out.
- **Such an app's request without PKCE must ask for `openid` and send a `nonce`** (RFC 9700 §2.1.1), and is refused otherwise. It may still use PKCE, and then the verifier is checked as before. A token request that sends a `code_verifier` for a code issued without a challenge is refused, so a challenge can't be stripped on the way.
- **`examples/idp_conformance`** is a test server for the OpenID conformance suite, with an HTML sign-in page for one seeded person and an HTML consent page, and a nightly workflow runs the Basic OP and Config OP plans against it.
