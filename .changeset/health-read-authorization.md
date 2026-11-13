---
"leptoskit_plugin_health": patch
---

Report HealthKit read permissions as unknown rather than deriving an exact read mask from write authorization. An empty inexact mask must not prevent an app from reading records the owner has authorized.
