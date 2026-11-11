---
"release": patch
---

# Bundled SQLite is 3.54.1

`leptoskit_db`'s `sqlite` feature, `leptoskit_db_schema`, `leptoskit_deploy_console`, and `leptoskit_plugin_sqlite` bundle SQLite through `rusqlite`, which moves from 0.40.2 to 0.41.1 so that `libsqlite3-sys` moves from 0.38.2 to 0.39.1 and the bundled SQLite from 3.53.2 to 3.54.1. Before 3.54.0, SQLite could return uninitialized page-cache memory when a process opened a database alongside a `-wal` file written with a different page size (CVE-2026-119375, RUSTSEC-2026-0459). Nothing in leptoskit's own API changed; `rusqlite` stays an implementation detail.
