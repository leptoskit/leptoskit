---
"leptoskit_cloud": patch
---

# A platform hostname is held until its removal is reported

Deleting an environment, or the project that holds it, used to free its platform hostname at once, while the cluster could still serve it until its worker finished the removal. Another organisation could take the hostname in that window and be served at it to the environment's former visitors. A placement now outlives its environment, marked `removing`, and keeps the hostname until the cluster's worker reports the removal done or an operator settles it. A placement being removed isn't counted as an environment its cluster runs, so it doesn't send new work to that cluster.

The `placement_claims` migration (`20261125T222120_placement_claims` on SQLite, `20261125T222243_placement_claims` on Postgres) adds `environment_placements.removing`, `false` for every row a database already has, and removes the foreign key that deleted a placement with its environment. Its reverse restores that foreign key, so it fails while a removal is still pending.
