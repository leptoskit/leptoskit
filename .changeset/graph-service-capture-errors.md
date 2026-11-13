---
"release": patch
---

# Report missing service metadata

Return a service capture diagnostic when the selected or workspace manifest is absent from resolved Cargo metadata, before attempting to read its package version. This keeps malformed or inconsistent source resolution from panicking during graph builds.
