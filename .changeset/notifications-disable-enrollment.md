---
leptoskit_notifications: patch
leptoskit_service_worker: patch
---

Disable only the authenticated person's service enrollment, preserving the browser subscription and permission for reuse. Require a successful revoke acknowledgement with a stable retry key; never infer success from an endpoint list. Remove the unused endpoint-matching unsubscribe helper.
