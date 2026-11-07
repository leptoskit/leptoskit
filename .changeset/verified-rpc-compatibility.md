---
"release": patch
---

# Bind supported client contracts to verified releases

`infra build-verified` can compare the reproduced executable's RPC export with independently pinned published contracts. Version 3 descriptors retain the exact specs and recompute the structural comparison when consumed. Version 2 operator trust files require a nonempty set of supported contract pins across deployment, reuse, rollback, and recovery; publishers cannot substitute their own baseline. Legacy evidence remains readable and explicitly unchecked. This checks declared wire shapes, not runtime behavior or authorization compatibility.
