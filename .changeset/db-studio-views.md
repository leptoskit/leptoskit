---
"release": minor
---

# Browse views in the database studio

`leptoskit db studio` now lists the app's `#[db::view]`s under its tables, materialized ones included, and shows their rows with the same filters, sorting, and paging, read-only. Each view's own `select` policy applies, and with `policies = "rls"` Postgres also reads a plain view's tables as the viewer you picked. A materialized view shows the rows of its last refresh. The schema page shows each view's columns and query.
