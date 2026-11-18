---
leptoskit_cloud_runtime: minor
---

Reserve one surge pod for every process that rolls, and expose `Quota::for_processes` so control-plane quotas match runtime validation. Processes with persistent volumes use Recreate and need no surge capacity.
