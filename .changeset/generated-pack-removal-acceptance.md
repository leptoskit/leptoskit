---
"release": patch
---

# Check pack removal in generated apps

The required generated database journey now checks accidental and explicit schema-pack removal against real migration files and persisted notes. Python with SQLite is required rather than silently skipping the data check. Migration consolidation reports a missing pack before generic table drift so the required removal intent is clear.
