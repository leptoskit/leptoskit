---
"release": minor
---

# The dev studio follows branch switches

`leptoskit dev --studio` now follows `git switch`: when `dev` moves to the new branch's database, it stops the studio first, so Postgres can copy the main branch's database for a branch that hasn't got one yet, then starts the studio again over the branch's database and prints its new address. You no longer need to restart `dev` to see the branch's rows.
