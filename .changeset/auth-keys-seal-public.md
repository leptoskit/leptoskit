---
"leptoskit_auth": minor
---

# Make `Keys::seal` and `Keys::open` public

Another crate can now seal bytes under a set of rotating keys, bound to the context parts it names, the way sessions and codes already are. `Keys::shares_key_with` says whether two sets hold the same key under any name, so a server can refuse to use one key for two jobs.
