---
"release": minor
---

# Hide private values in the studio unless revealed

`leptoskit db studio` now shows `private` columns as `‹private›`, on table and view pages and in the audit log's records of rows, so a shared or recorded screen doesn't give them away. Start it with `--reveal` to see them, or `leptoskit dev --studio --reveal`; every page then says that private values are revealed. `secret` columns never show. The studio no longer filters or sorts by a column it hides, `secret` ones included, since matching or ordering by one would tell its values apart.
