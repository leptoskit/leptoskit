---
"leptoskit_cloud": minor
---

# Add environment variables and the secrets vault

`leptoskit_cloud::config` adds `variables`, `set_variable`, `unset_variable`, `secrets`, `set_secret` and `unset_secret`, behind a new `manage_config` permission for owners, admins and developers. Variables are written into the next deployment's release; secrets are named in it and their values are sealed per organisation with AES-256-GCM-SIV, bound to where they were stored, and leave the control plane only in a cluster worker's claim of that release, with a lease row for each.

A release whose secrets can't be read fails with the new `secret_unavailable` blocker. The control plane needs `LEPTOSKIT_CLOUD_VAULT_KEYS`, and `ControlConfig` gains `vault`.
