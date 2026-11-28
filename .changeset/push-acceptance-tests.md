---
"leptoskit_notifications": patch
---

# Count a test push as a push

A test send wrote nothing, so the acceptance ledger a hosting plan meters went without it: a free plan's pushes could be tested past its allowance, and the pause it earns never came. An accepted test push now writes its row in `push_acceptances`, with an id of its own since a test has no delivery, and a push the provider refuses writes nothing. No delivery, send, or inbox row comes with it.
