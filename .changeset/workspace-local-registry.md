---
"leptoskit_config": patch
"leptoskit_cli": patch
---

# Accept managed registries with explicit ports

Managed image configuration accepts a registry hostname with a numeric port and an image repository under that registry. Invalid ports, embedded credentials, paths in registry authorities, and tagged image repositories remain rejected.

Verified image packaging accepts `--service <infra.name>` to include Kamal’s required service label in the independently pinned image configuration. Reusing an image checks the label along with the rest of that configuration.

Deployment accepts Docker’s null defaults for absent Podman `Cmd`, `OnBuild`, and `Volumes` inspection fields while still requiring the exact content-addressed image ID. Non-null changes remain rejected.
