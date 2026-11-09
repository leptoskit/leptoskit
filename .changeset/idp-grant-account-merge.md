---
"release": patch
---

# Merge accounts in apps with the identity provider

An app that links `leptoskit_idp` can now merge two accounts and bring the other account's data along. Before, `idp.grants` declared no `on_account_merge` strategy, so `lk db check` failed and "bring its data" merges were refused. A merge now moves the absorbed account's grants to the account that stays. If both accounts granted the same app, the surviving account keeps its own grant and the absorbed account's grant is removed along with that account.
