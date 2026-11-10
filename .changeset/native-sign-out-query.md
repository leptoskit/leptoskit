---
"leptoskit_auth": patch
---

# Native sign-out clears the session query when offline

`sign_out_mutation()` now clears a native app's session query when it forgets its local tokens, including when the server is unreachable or refuses the request. The mutation still reports that error: local sign-out does not claim the server revoked the session. A browser keeps its session query on failure because its cookie remains until the server removes it.
