---
"release": patch
---

# Install valid scheduled backup units

Managed backups now generate separate systemd directives for the recovery command, writable state paths, and timer installation. Previously, malformed newline escapes joined these settings to preceding lines and could prevent scheduled backups from running. The host verifies both generated units before initializing the backup repository or enabling the timer.

After upgrading, run `lk infra backups install` for existing managed deployments to replace their generated units.
