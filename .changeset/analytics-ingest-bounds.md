---
"release": patch
---

# Bound analytics batch decoding and extreme clocks

Analytics ingestion now limits the memory used when decoding binary batches, including repeated dictionary references, before allocating strings or collections. Extreme client clock offsets are clamped instead of overflowing timestamp arithmetic. The client core is also included in the regular Wasm lint checks.
