---
"release": minor
---

# Reproduce website assets before verified packaging

Add an explicit versioned website recipe that runs the production web pipeline offline in a digest-pinned builder, compares complete asset trees, and packages the checked backend and website bytes. Website descriptors bind paths, modes, hashes and build inputs to independent approvals and image-bound deployment policies; incomplete website evidence fails closed.
