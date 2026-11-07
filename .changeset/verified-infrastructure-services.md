---
"release": patch
---

# Verified backends can use infrastructure services

Verified backend packaging now distinguishes application services from the separate cache and relay containers configured under `infra.services`. Those infrastructure settings no longer incorrectly reject a backend-only image. Additional application executables still require their own evidence and remain unsupported by this packaging profile. The descriptor verifies the backend image; it does not attest the infrastructure containers or their runtime configuration.
