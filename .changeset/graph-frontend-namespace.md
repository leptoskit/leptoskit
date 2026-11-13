---
leptoskit_config: patch
leptoskit_cli: patch
---

# Keep graph placement separate from application frontends

Graph artifact placement uses `[infra.frontends.<name>]`. Application build frontends keep their existing top-level `[frontends.<name>]` contract. This avoids colliding configuration types when named application targets and deployment graphs are used in the same toolkit. Graph unit identifiers such as `frontends.docs` and all ownership, route and recovery checks are unchanged.
