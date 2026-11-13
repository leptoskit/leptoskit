---
"release": patch
---

# Keep plugin package readme paths portable

The web-auth and passkeys crates explicitly name their existing `readme.md` files. Cargo no longer adds an automatically detected `README.md` alongside the lowercase file, so their published archives extract on case-insensitive filesystems.
