---
"release": patch
---

# SQLite live feeds hear other processes' writes

On SQLite, `Db::changes()` feeds and the job workers' wake-up now notice changes another process commits to the same database file within a second. Before, they heard of them only when their own process next wrote. Postgres is unchanged: it already wakes every server at commit.
