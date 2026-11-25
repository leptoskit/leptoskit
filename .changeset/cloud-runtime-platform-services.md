---
"leptoskit_cloud_runtime": minor
---

# A release can route to a service the platform runs

`ReleaseSpec.platform` names services the platform runs outside the environment, such as its cluster's shared notifications hub. A route may name one instead of a process. The manifests give each a bridge in the environment's namespace, an `ExternalName` Service with the service's port, route to it, and open the namespace's egress to that service's namespace on that port and nothing more of it. A spec without platform services serializes as before, and one written before they existed reads with none.
