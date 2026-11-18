---
"leptoskit_cloud": minor
---

# Add custom domains to the control plane

`leptoskit_cloud::domains` adds `domains`, `attach_domain`, `verify_domain` and `detach_domain`. A domain is proved with a TXT record read over DNS-over-HTTPS, a verified hostname belongs to one environment in the whole cloud, and the environment's next release routes it with a certificate of its own. A detached domain keeps its hostname until a deployment without it is live, and a rollback never routes a domain the environment no longer holds.

`ControlConfig` gains `resolver`, and the control plane reads `LEPTOSKIT_CLOUD_DNS_RESOLVER`.
