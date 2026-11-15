---
"leptoskit_analytics": minor
---

# Retention cohorts every period; event definitions

- **Retention cohorts.** A retention query now answers one cohort for every period in its range, oldest first, even when nobody started in a period (its size is 0). `ActorPoint::RetentionCell { start_period, .. }` therefore indexes `cohorts` directly. Each cohort is followed for `periods` periods or until the range ends, so later cohorts have shorter `returns`. First-time cohorts look at the start event's whole history, so someone who first did it before the range doesn't count as new inside it.
- **Wire rename.** A retention query's first-time or recurring choice now travels as `retention_kind`. Before, it travelled as `kind`, which collided with `Query`'s own tag: saving a retention insight failed.
- **`event_definitions`.** This app-only operation lists every event name the service has seen, with:
  - its volume
  - when it was first and last seen
  - the fields of its registered Rust type, if it has one
  - the properties its recent events carried

  The admin's builders read it.
