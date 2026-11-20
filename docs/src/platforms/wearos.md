# Wear OS

<Callout kind="warning" title="Experimental">

Wear OS support is early. The Rust watch API and Kotlin host stubs exist; APK packaging, Play's Wear track upload, and live tile or complication updates do not. Use it to design Wear surfaces, not to ship.

</Callout>

On Wear OS, leptoskit does not pretend a phone activity is a watch. You describe a `WatchApp` with pages, tiles, and complications. The experimental host under `hosts/wearos` names the Wear entry points a future generated app will bind: a watch activity, a `TileService`, and a complication data source.

## What works and what doesn't

| Works                                                                             | Doesn't work yet                            |
| --------------------------------------------------------------------------------- | ------------------------------------------- |
| `WatchApp` / `WearableApp` surfaces, budgets, and `WearOsHost` recording tests    | Play Store Wear track publishing automation |
| Android Gradle APK packaging with `android.hardware.type.watch` manifest transform | Code push updates on Wear OS                |
| Full Android runtime plugins (`wearable`, `health`, `watch-sync`, `ble`, `location`, `battery`, `thermal`, `background`) | Automated Wear OS emulator harness in CI |
| Samsung Galaxy Watch (Wear OS / One UI Watch) + optional Samsung Health Sensor SDK hook | |

## Build

With an app-owned `platforms/wearos` (or `platforms/android`) Gradle host and `[native.wearos]` configured:

```sh
leptoskit native prepare wearos
leptoskit native build wearos
```

The build transforms the merged `AndroidManifest.xml` to add `<uses-feature android:name="android.hardware.type.watch" android:required="true"/>`, `com.google.android.wearable.standalone`, and optional Tile and Complication service entries, then packages the Wear OS APK with Gradle. Install it on a watch over ADB (`adb -s <watch> install ...`).
