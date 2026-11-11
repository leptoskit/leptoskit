---
"release": patch
---

Build the public and hosted website clients with a size-optimized Rust standard library during release packaging. This keeps the initial download within the existing 180 KiB budget while preserving the live example and the console's lazy loading.
