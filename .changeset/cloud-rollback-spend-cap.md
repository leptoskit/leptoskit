---
"leptoskit_cloud": patch
---

# Refuse `rollback` at the spend cap, as `deploy` is

Once an organisation's usage for the month reaches its spend cap, `rollback` answers `spend_cap_reached` and makes no deployment, as `deploy` does. Until now a rollback could still put an earlier release live past the cap, and bring back a larger scale than the one running. What already runs keeps running: nothing is slept, stopped or deleted. Under the cap, rolling back works as before.
