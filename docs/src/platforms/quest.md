# Meta Quest

<Callout kind="warning" title="Experimental">

The Quest build is new. It produces an APK with everything Horizon OS asks of a VR app, but it hasn't been installed on a headset yet. What the headset shows depends on the app drawing through OpenXR; see below.

</Callout>

Meta Quest headsets run Horizon OS, which is Android. A leptoskit app's Android project becomes a Quest app with one flag: the same Kotlin host, the same Rust, and the same plugins, packaged with the manifest a headset needs.

```sh
leptoskit build --platform quest      # an APK in hosts/android/app/build/outputs/apk
leptoskit dev --platform quest        # install and run on a connected headset
```

Add `"quest"` to `app.platforms` next to `"android"`. The Quest build uses the app's `hosts/android` project, so build one platform at a time: each build regenerates `.leptoskit/android` for its own destination.

## Connect a headset

Turn on Developer Mode for the headset in the Meta Horizon app on your phone, connect it over USB, and accept the debugging prompt inside the headset. `adb devices` then lists it, and `leptoskit dev --platform quest` installs onto every connected Quest. There is no Quest emulator.

## Settings

```toml
[native.quest]
immersive = true
supported-devices = ["quest2", "questpro", "quest3", "quest3s"]
hand-tracking = "optional"
passthrough = true
```

| Setting             | Default             | What it does                                                                                                                                   |
| ------------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `immersive`         | `true`              | Launches into the headset's immersive mode. With `false`, the app opens as a 2D panel showing its ordinary interface, as any Android app does. |
| `supported-devices` | every current Quest | The headsets the Horizon Store offers the app on.                                                                                              |
| `hand-tracking`     | `"optional"`        | `"optional"` uses hands where the player has them on, `"required"` hides the app from players without hand tracking, `"off"` leaves hands out. |
| `passthrough`       | `true`              | Declares passthrough, so `ImmersiveAr` sessions can show the room.                                                                             |

## What the build adds

On top of the app's own manifest, the Quest build declares:

- head tracking (`android.hardware.vr.headtracking`), required;
- the OpenXR loader's permissions and the runtime broker it queries;
- hand tracking (`oculus.software.handtracking` and its permission) and passthrough (`com.oculus.feature.PASSTHROUGH`), as configured;
- the supported headsets (`com.oculus.supportedDevices`);
- for immersive apps, the `com.oculus.intent.category.VR` and OpenXR `IMMERSIVE_HMD` launcher categories;
- a full-screen, landscape activity with no title or status bar and no picture-in-picture, that handles every configuration change itself so a session survives it.

The [`xr` plugin](../guide/35-headsets.md) brings the Khronos OpenXR loader with it.

## What works and what doesn't

| Works                                                                   | Doesn't work yet                                                                                               |
| ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| Building an APK with the Quest manifest                                 | Running it on a headset hasn't been verified                                                                   |
| Installing onto a connected headset with `leptoskit dev`                | Drawing the app's 2D interface inside an immersive session: immersive mode shows only what the app draws in 3D |
| Every runtime plugin the Android build has                              | Publishing to the Horizon Store with `leptoskit publish`                                                       |
| The `xr` plugin's headset sessions, controllers, hands, and passthrough | Release signing for the Horizon Store beyond the Android upload key                                            |

In immersive mode the headset shows only what the app draws through OpenXR, so a Quest game draws its world, and its menus, with the 3D renderer. Set `immersive = false` for an app whose interface is its ordinary screens: it runs in a window in the player's space.
