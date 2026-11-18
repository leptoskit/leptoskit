---
"leptoskit_auth": minor
---

# Make `Keys::seal` and `Keys::open` public

Another crate can now seal bytes under a set of rotating keys, bound to the context parts it names, the way sessions and codes already are.
