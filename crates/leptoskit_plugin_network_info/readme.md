<h1 align="center">leptoskit_plugin_network_info</h1>

<p align="center">
  <strong>leptoskit_plugin_network_info</strong> is the leptoskit plugin for the Wi-Fi name, BSSID, and local address details.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_network_info"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_network_info"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_network_info"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_network_info.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_network_info"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_network_info/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

A port of Flutter's [`network_info_plus`](https://github.com/fluttercommunity/plus_plugins/tree/main/packages/network_info_plus). One call returns a `Snapshot` with every field upstream has a getter for:

| `network_info_plus` | `Snapshot` field | Type                       |
| ------------------- | ---------------- | -------------------------- |
| `getWifiName`       | `ssid`           | `Option<Ssid>` (raw bytes) |
| `getWifiBSSID`      | `bssid`          | `Option<Bssid>`            |
| `getWifiIP`         | `ipv4`           | `Option<Ipv4Addr>`         |
| `getWifiIPv6`       | `ipv6`           | `Option<Ipv6Addr>`         |
| `getWifiSubmask`    | `subnet_mask`    | `Option<Ipv4Addr>`         |
| `getWifiBroadcast`  | `broadcast`      | `Option<Ipv4Addr>`         |
| `getWifiGatewayIP`  | `gateway`        | `Option<Ipv4Addr>`         |

Every field is `None` when the platform withholds it or has no such value. Unlike upstream, the plugin never passes on placeholders such as Android's `<unknown ssid>` or `02:00:00:00:00:00`; the Rust decoder refuses them.

An SSID is 1 to 32 arbitrary bytes and need not be UTF-8, so `Ssid::as_bytes` returns exactly what the platform reported and `Ssid::to_str` returns text only for valid UTF-8. macOS and Android report raw bytes; iOS hands over an already-decoded string, which arrives as its UTF-8 bytes.

### What each platform needs

The plugin never asks for permission. The SSID and BSSID need location authorization everywhere that reports them; request it with the `location` plugin (`guest::Location::request_permission(true)`), then read the snapshot again. Addresses and the gateway need nothing beyond the declarations below.

- **iOS:** `NEHotspotNetwork.fetchCurrent` answers only when the app has the **Access Wi-Fi Information** entitlement (`com.apple.developer.networking.wifi-info`) in its provisioning profile and entitlements file, plus location authorization (or a hotspot or VPN configuration the app installed). Add `NSLocationWhenInUseUsageDescription` to `ios/Info.plist` for the `location` plugin's prompt. Without them iOS answers nothing and the SSID and BSSID are `None`. Addresses come from `getifaddrs` for the Wi-Fi interface that `NWPathMonitor` names, and the gateway from that interface's default route.
- **Android:** the plugin's metadata adds the normal `ACCESS_NETWORK_STATE` and `ACCESS_WIFI_STATE` permissions. The SSID and BSSID also need `ACCESS_FINE_LOCATION` granted at run time; selecting the `location` plugin declares it, and `request_permission(true)` asks for it. Without it Android answers `<unknown ssid>` and `02:00:00:00:00:00`, which arrive as `None`. Addresses, the mask and the gateway come from the Wi-Fi network's `LinkProperties`, even when a VPN or cellular network is the default; the broadcast address is computed from the prefix.
- **macOS:** CoreWLAN reports the SSID and BSSID only after the app has location authorization; the `location` plugin needs `NSLocationUsageDescription`, and the location entitlement in a sandboxed app. Addresses and the gateway work as on iOS.
- **Desktop Rust (macOS, Windows, Linux):** `desktop::snapshot()` and the `desktop::NetworkInfo` adapter report the addresses of the interface carrying the IPv4 default route, which may be wired, read with `if-addrs` and the routing table (`/proc/net/route`, a `PF_ROUTE` sysctl, `GetIpForwardTable2`). The SSID and BSSID are always `None`; generated macOS apps use the Swift plugin, which reads them.
- **Browser:** `web::snapshot()` returns `Unavailable`; browsers expose none of this to pages.
- **HarmonyOS:** no ArkTS implementation yet.

The wire snapshot is 80 bytes: presence bits, the SSID length and 32 SSID bytes, the BSSID, then the addresses in network byte order, with absent fields all zero. Rust, Swift and Kotlin tests assert the same golden bytes.

## How it fits

<!-- {=p_networkInfoSummary} -->

Add `leptoskit_plugin_network_info` as a direct dependency and select the `network-info` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Guests enable the `guest` feature and reuse `guest::NetworkInfo`. One snapshot reports the Wi-Fi name (SSID, as raw bytes), BSSID, IPv4 and IPv6 addresses, subnet mask, broadcast address and gateway, as Flutter's `network_info_plus` does. Each field is `None` when the platform withholds it; the plugin never returns placeholders and never asks for permission. The SSID and BSSID need location authorization, which the `location` plugin requests, and on iOS the Access Wi-Fi Information entitlement too. Browsers return `Unavailable`.

<!-- {/p_networkInfoSummary} -->

`leptoskit_plugin_network` answers "am I online, and over what?"; this crate answers "which Wi-Fi network, and at what address?". They are separate plugins so an app that only checks connectivity never declares Wi-Fi permissions or hands location-grade identifiers to its guest code.

## Use it

```toml
# In a native host, select `network-info` (and `location`, for the SSID) in `native.plugins`.
[dependencies]
leptoskit_plugin_network_info = { version = "0.3", features = ["guest"] }
```

```rust,ignore
let info = leptoskit_plugin_network_info::guest::NetworkInfo::new()?;
let snapshot = info.snapshot().await?;

if let Some(ssid) = snapshot.ssid {
	println!("on {}", ssid.to_string_lossy());
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_network_info)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
