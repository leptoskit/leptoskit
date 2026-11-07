---
"leptoskit_db": patch
---

# Refuse ambiguous consolidated rollback without panicking

Rollback now returns a migration-history error when an adopted consolidation leaves more applied history rows than the current registry contains. It stops before reverse SQL or history changes instead of indexing beyond the registry. Ordinary rollback of an intact registry prefix is unchanged.
