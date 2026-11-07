---
"release": patch
---

# Reject future backup timestamps when reporting health

Managed backup status no longer reports a verification timestamp in the future as healthy. A successful restore check must have a nonnegative age within the configured freshness window, and any later recorded backup failure still makes the status unhealthy.
