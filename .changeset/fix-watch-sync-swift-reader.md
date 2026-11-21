---
"leptoskit_plugin_watch_sync": patch
---

# Fix WireReader storage property name in WatchSyncPlugin.swift

Renamed `WireReader.bytes` storage property to `buffer` to avoid a Swift redeclaration error with `mutating func bytes()`.
