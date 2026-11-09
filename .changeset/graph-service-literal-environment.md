---
"release": patch
---

# Deliver declared literal settings to graph services

Graph service capture and release now carry non-secret `env` settings from package metadata and project overrides into the process contract. Empty values, Unicode, spaces, quotes, backslashes, dollar signs and percent signs are preserved. Changes are bound to the captured configuration, saved release plan and installed process record, including rollback. A service cannot override its declared listen variable through `env`.

These values are public release inputs and appear in receipts and unit files. Secret delivery remains unsupported. Unsupported control characters and oversized environments fail before release writes rather than being silently changed or omitted.
