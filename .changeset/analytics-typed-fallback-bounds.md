---
"leptoskit_analytics": patch
---

Preserve typed fallback context and queue bounds

Keyed retries of unknown typed schemas retain their original context, UUID and field precedence. Entries expanded past a batch or queue byte limit are dropped and counted, and total queue accounting includes retained contexts. Catalogue validation also checks known vector element types recursively.
