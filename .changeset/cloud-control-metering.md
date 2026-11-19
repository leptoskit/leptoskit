---
"leptoskit_cloud": minor
---

# Count usage, limit plans and cap spending

`leptoskit_cloud::billing` adds `plans`, `usage` and `set_spend_cap`. The control plane now counts build minutes and compute seconds per project and calendar month, and `usage` prices a month on the organisation's plan with the operator's catalogue. Once a month's usage reaches the spend cap, `deploy` answers `spend_cap_reached`; nothing running is stopped.

A plan in the price catalogue can carry `limits` (`organizations`, `projects`, `custom_domains`) and a `spend_cap`; `create_organization`, `create_project` and `attach_domain` answer `quota_exceeded` at a limit. A control plane now refuses to start with a catalogue that has no `hobby` plan. `Permission` gains `ViewBilling` and `ManageBilling`, and `pricing::Plan` gains `limits` and `spend_cap`, so code that builds a `Plan` by hand must set them. Nothing charges anyone yet.
