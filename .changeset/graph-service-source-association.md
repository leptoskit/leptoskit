---
"release": patch
---

# Bind service paths to captured manifests

Check that a path-based service's selected Cargo manifest is the manifest at its declared directory before normalizing portable evidence, and recheck that association when consuming a receipt. Bind the recorded lock to its workspace directory and require its local package/version entry. Replacing the selected manifest with another package's identical bytes no longer establishes the declared source identity.
