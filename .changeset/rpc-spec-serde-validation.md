---
"release": patch
---

# Validate RPC specifications through every JSON reader

Deserializing `ApiSpec` with Serde now validates specification versions and migrates older nullable arguments in the same way as `ApiSpec::from_json`. Unsupported versions fail instead of bypassing the compatibility checks. Version 4 keeps required nullable arguments required.
