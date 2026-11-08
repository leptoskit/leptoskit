---
"leptoskit_plugin_network_info": minor
---

# Wi-Fi name and local addresses

The new `leptoskit_plugin_network_info` crate, runtime name `network-info`, ports Flutter's `network_info_plus`. `guest::NetworkInfo::snapshot` returns the Wi-Fi network's name (SSID) and BSSID, and its IPv4 address, IPv6 address, subnet mask, broadcast address, and gateway, for apps that set up or find devices on the local network.

Every field is `None` when the platform withholds it; the plugin never passes on stand-ins such as Android's `<unknown ssid>` or `02:00:00:00:00:00`, and never asks for permission. The SSID and BSSID need location permission, which the `location` plugin requests; on iOS they also need the Access Wi-Fi Information entitlement. An SSID is up to 32 bytes that need not be valid text: `Ssid::as_bytes` returns them as reported, and `Ssid::to_str` returns text when they are UTF-8.

iOS reads `NEHotspotNetwork`, macOS CoreWLAN, and Android `WifiManager` with the Wi-Fi network's `LinkProperties`. Desktop Rust reports the addresses and gateway of the interface the default route leaves through, without a name, and the browser returns `Unavailable`. It is a separate plugin from `network` so an app that only checks connectivity never declares Wi-Fi permissions or hands Wi-Fi identifiers to its guest code.
