# watchOS

<Callout kind="warning" title="Experimental">

watchOS support is early. The Rust watch API and Swift host stubs exist; Xcode packaging, simulator install, and live complication or WidgetKit updates do not. Use it to design watch surfaces, not to ship.

</Callout>

On watchOS, leptoskit models pages, Smart Stack glances, and complications as first-class surfaces. A phone UIKit tree is not reused. The experimental host under `hosts/watchos` names the WatchKit app, WidgetKit glance, and complication entry points a future generated app will bind.

## What works and what doesn't

| Works                                                                                               | Doesn't work yet                                                          |
| --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `WatchApp` / `WearableApp` surfaces, budgets, and `WatchOsHost` tests                               | Automated watchOS simulator harness in CI                                 |
| `.leptoskit/watchos` plugin registrant, `Info.plist`, entitlements, and `WatchHost.xcconfig`        | Code push updates on watchOS                                              |
| Apple runtime plugins (`wearable`, `health`, `watch-sync`, `ble`, `location`, `battery`, `thermal`) | App Store Connect watchOS upload automation                               |
| `leptoskit native build watchos` via `xcodebuild -sdk watchos` on macOS                             | An app's own [fonts](../guide/42-fonts.md): text draws in the system font |

## Build

On macOS with Xcode and an app-owned `platforms/watchos` host:

```sh
leptoskit native prepare watchos
leptoskit native build watchos
```

`prepare` writes `.leptoskit/watchos/GeneratedPlugins.swift`, `Info.plist` (including `WKApplication`, `WKRunsIndependentlyOfCompanionApp`, and `WKBackgroundModes`), `WatchApp.entitlements`, and `WatchHost.xcconfig`, then `build` invokes `xcodebuild -sdk watchos`.
