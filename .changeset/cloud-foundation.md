---
"leptoskit_cloud": minor
"leptoskit_config": minor
---

# The groundwork for LeptosKit Cloud

`leptoskit.toml` accepts a `[cloud]` table that says where LeptosKit Cloud should run your app and how big: a project, an optional region, and per-environment size, replicas, sleeping when idle and custom domains, plus the database's size and backups. Config checks it as it loads, with errors that name the table and key, such as a SQLite app asking for more than one replica, or sleeping while it serves live queries. The new `leptoskit_cloud` crate holds the shared types behind it: slugs, sizes, deployment states, the source archive a deploy will upload (deterministic, with secret files refused), and a monthly cost estimate for a `[cloud]` table. The cloud itself doesn't run yet, so nothing deploys there today.
