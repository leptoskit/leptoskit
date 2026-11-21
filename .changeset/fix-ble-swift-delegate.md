---
"leptoskit_plugin_ble": patch
---

# Fix Swift 6 delegate isolation in BlePlugin.swift

Adopted `@preconcurrency CBCentralManagerDelegate` and `@preconcurrency CBPeripheralDelegate` on `@MainActor BlePlugin` (whose central manager runs on `.main`) to resolve Swift 6 `sending` data-race errors on `CBPeripheral`.
