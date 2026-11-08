# Meta Quest

<Callout kind="warning" title="Experimental">

The Quest build is new. It produces an APK with everything Horizon OS asks of a VR app and draws the app's 3D scene into the headset, but it hasn't been installed on a headset yet.

</Callout>

Meta Quest headsets run Horizon OS, which is Android. A leptoskit app's Android project becomes a Quest app with one flag: the same Kotlin host, the same Rust, and the same plugins, packaged with the manifest a headset needs.

```sh
leptoskit build --platform quest      # an APK in hosts/android/app/build/outputs/apk
leptoskit dev --platform quest        # install and run on a connected headset
leptoskit build --release --platform quest  # a signed APK in .leptoskit/release/quest
```

A release is signed with your Android upload key, from the same variables as an Android release; see [Signing a release](../guide/24-signing.md#android-and-meta-quest).

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

| Setting             | Default             | What it does                                                                                                                                                                                    |
| ------------------- | ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `immersive`         | `true`              | Launches into the headset's immersive mode. With `false`, the app opens as a 2D panel showing its ordinary interface, as any Android app does, and can still open an immersive session (below). |
| `supported-devices` | every current Quest | The headsets the Horizon Store offers the app on.                                                                                                                                               |
| `hand-tracking`     | `"optional"`        | `"optional"` uses hands where the player has them on, `"required"` hides the app from players without hand tracking, `"off"` leaves hands out.                                                  |
| `passthrough`       | `true`              | Declares passthrough, so `ImmersiveAr` sessions can show the room.                                                                                                                              |

## What the build adds

On top of the app's own manifest, the Quest build declares:

- head tracking (`android.hardware.vr.headtracking`), required;
- the OpenXR loader's permissions and the runtime broker it queries;
- hand tracking (`oculus.software.handtracking` and its permission) and passthrough (`com.oculus.feature.PASSTHROUGH`), as configured;
- the supported headsets (`com.oculus.supportedDevices`);
- for immersive apps, the `com.oculus.intent.category.VR` and OpenXR `IMMERSIVE_HMD` launcher categories;
- a full-screen, landscape activity with no title or status bar and no picture-in-picture, that handles every configuration change itself so a session survives it;
- the immersive activity the `xr` plugin opens for a headset session;
- for panel apps, the `com.oculus.intent.category.2D` launcher category and Horizon OS v69 as the target, which lets a panel open an immersive activity.

The [`xr` plugin](../guide/35-headsets.md) brings the Khronos OpenXR loader with it.

## Immersive sessions from a panel

A panel app keeps its ordinary screens and opens the headset for one part of it: a game whose menus are a panel and whose hunts fill the room. Start an `ImmersiveAr` or `ImmersiveVr` session from your code as on any headset, and choose the 3D scene it shows with `present`:

```rust
use leptoskit_xr::Features;
use leptoskit_xr::Mode;
use leptoskit_xr::ReferenceSpace;
use leptoskit_xr::SessionConfig;
use leptoskit_xr::Xr;

async fn enter_the_hunt(scene: u32) -> Result<leptoskit_xr::Session, leptoskit_xr::Error> {
	let session = Xr::new()?
		.start(
			SessionConfig::new(Mode::ImmersiveAr)
				.reference_space(ReferenceSpace::LocalFloor)
				.prefer(Features::CONTROLLERS | Features::HANDS | Features::PASSTHROUGH),
		)
		.await?;
	session.present(Some(scene)).await?;
	Ok(session)
}
```

The headset leaves the panel for an immersive view of the scene, with the room showing through where the scene draws nothing. Your code keeps running and receives the session's frames, controllers, and hands as usual. When you `stop` the session, or the player leaves it, the immersive view closes and the panel comes back in Home. The scene needs `leptoskit`'s `3d` feature; `leptoskit build --platform quest` draws it.

## What works and what doesn't

| Works                                                                   | Doesn't work yet                                                                                               |
| ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| Building an APK with the Quest manifest                                 | Running it on a headset hasn't been verified                                                                   |
| Installing onto a connected headset with `leptoskit dev`                | Drawing the app's 2D interface inside an immersive session: immersive mode shows only what the app draws in 3D |
| Every runtime plugin the Android build has                              | Publishing to the Horizon Store with `leptoskit publish`                                                       |
| A release APK signed with your upload key                               |                                                                                                                |
| The `xr` plugin's headset sessions, controllers, hands, and passthrough | Uploading the signed APK to the Horizon Store: use Meta's own tools                                            |
| Drawing a 3D scene into the headset, from a panel app too               | Running the immersive switch and the return to the panel on a headset hasn't been verified                     |

In immersive mode the headset shows only what the app draws through OpenXR, so a Quest game draws its world, and its menus, with the 3D renderer. Set `immersive = false` for an app whose interface is its ordinary screens: it runs in a window in the player's space.
