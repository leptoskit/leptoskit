---
"leptoskit_plugin_network": minor
---

# Every transport a connection uses

A network reading now lists every transport the active connection uses, as Flutter's `connectivity_plus` does, instead of naming only one. `Snapshot::transports` is a `Transports` set of `Transport` values — Wi-Fi, cellular, ethernet, Bluetooth, VPN, satellite, and other — so an Android app on a VPN over Wi-Fi sees both, and an app on satellite data sees satellite alongside cellular. `Snapshot::connection` still names the primary one, and the set always contains it.

Android reports every transport `NetworkCapabilities` lists, including Wi-Fi Aware as Wi-Fi and satellite on Android 15 and later. iOS and macOS report Wi-Fi, cellular, ethernet, and other from `NWPath`, plus satellite when the path is ultra-constrained (iOS and macOS 26 and later); they have no VPN or Bluetooth interface type, so a VPN tunnel shows as other. The browser maps `NetworkInformation.type`, and desktop Rust still infers no transport.

The wire snapshot grows from 32 to 36 bytes, and every side refuses transport bits it does not define. Rebuild the native app together with the guest; a guest built with this release can't read an older shell's snapshots, and the reverse.
