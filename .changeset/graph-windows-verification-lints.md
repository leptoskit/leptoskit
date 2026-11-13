---
leptoskit_cli: patch
---

# Keep graph verification checks portable

Graph directory creation only mutates its builder when applying Unix permissions. On other hosts, descriptor capability methods retain their instance API and continue refusing operations that require Unix metadata. Windows Clippy no longer rejects these unsupported-platform branches.
