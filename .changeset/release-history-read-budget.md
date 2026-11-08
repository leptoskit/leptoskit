---
"release": patch
---

# Keep shared release history readable

Managed deployment now checks the complete proposed release history against its 4 MiB read limit before booting applications or changing their database. Large verified descriptors and repeated evidence can otherwise make a successful deployment's history unreadable by the next release or recovery command. The final write is checked again; existing history is never silently truncated or discarded.

Pulled and running image references must contain exactly 64 lowercase hexadecimal SHA-256 characters after the configured repository name. A matching repository prefix alone no longer qualifies a malformed reference as a registry digest.

This guard prevents new unreadable history. It does not archive existing history, repair an already oversized file, or provide unlimited release retention.
