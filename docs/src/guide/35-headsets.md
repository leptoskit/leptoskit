# VR and AR on headsets

<Callout kind="warning" title="Experimental">

Headset support is new and has not yet run on a headset. The Quest build produces an APK with the manifest Horizon OS expects and draws your 3D scene into the headset, and the OpenXR backend builds for Android, Windows, and Linux. None of it has been checked on a Quest yet.

</Callout>

On a headset the whole view is the app's. A VR game builds a world around the player; a mixed-reality game shows the player's own room through the headset's cameras (passthrough) and puts creatures in it. Either way the app needs the head's pose for both eyes every frame, the controllers or hands, and where the floor is. The same `xr` plugin that runs [augmented reality on phones](34-augmented-reality.md) runs headsets through OpenXR, so one app can serve both.

## The modes

| Mode          | What the player sees                                       | Where                                                                 |
| ------------- | ---------------------------------------------------------- | --------------------------------------------------------------------- |
| `ImmersiveVr` | Only what the app draws.                                   | Meta Quest, Android XR headsets, PC headsets through OpenXR.          |
| `ImmersiveAr` | The room through passthrough, with the app's scene on top. | Meta Quest 3 and 3S, Quest Pro, Android XR, and see-through displays. |

Ask with `capabilities` and start the one you want, as on a phone:

```rust
use leptoskit_xr::Features;
use leptoskit_xr::Mode;
use leptoskit_xr::ReferenceSpace;
use leptoskit_xr::SessionConfig;
use leptoskit_xr::Xr;

async fn enter_the_room() -> Result<leptoskit_xr::Session, leptoskit_xr::Error> {
	let xr = Xr::new()?;
	let mode = xr
		.capabilities()
		.await?
		.first_of(&[Mode::ImmersiveAr, Mode::ImmersiveVr, Mode::PhoneAr, Mode::Orientation])
		.ok_or(leptoskit_xr::Error::Unavailable)?;

	xr.start(
		SessionConfig::new(mode)
			.reference_space(ReferenceSpace::LocalFloor)
			.prefer(Features::CONTROLLERS | Features::HANDS | Features::PASSTHROUGH)
			.frame_rate(72),
	)
	.await
}
```

On a headset, `LocalFloor` puts y = 0 on the floor from the first frame, and `World` uses the play area the player set up.

## Show a scene in the headset

The headset shows a [3D scene](33-3d-scenes.md): tell the session which one with `present`, by the id its `Scene3d` view uses. Each eye sees it from where the player's eye is, and in `ImmersiveAr` the room shows through wherever the scene draws nothing:

```rust
use leptoskit::scene3d::SceneId;

const ROOM: SceneId = SceneId(1);

async fn show_the_room(session: &leptoskit_xr::Session) -> Result<(), leptoskit_xr::Error> {
	session.present(Some(ROOM.0)).await
}
```

Place models in the session's world coordinates, the same ones the frames' `viewer` and `inputs` use; the session's eyes replace the scene's own camera. On a phone `present` changes nothing, since the phone shows the scene in its `Scene3d` view; in the browser the page draws the WebXR session itself.

## Controllers and hands

Each frame's `inputs` lists what the player holds or shows:

- A **controller** has a `target_ray` (aim along its −Z), a `grip` pose for drawing it in the hand, `buttons` (trigger, squeeze, then A/X and B/Y on Meta's Touch controllers), and `axes` (the thumbstick, x then y, up positive).
- A **hand** has the 26 joints of OpenXR's and WebXR's hand model in `hand`, each with a pose and a radius, and aims from the palm.

Aim a hit test from a controller with `HitTestRequest::ray(input_ray)`, where `input_ray` is a `Ray` from the target ray's position along its forward direction.

## Meta Quest

`leptoskit build --platform quest` builds the app's Android project as a Quest app; `leptoskit dev --platform quest` installs it on a headset connected over USB. See [Meta Quest](../platforms/quest.md) for the settings, what the build adds to the manifest, and what works today.

## On each platform

| Platform                                  | What it does                                                                                                                                                                                          |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Meta Quest                                | OpenXR through the Khronos loader: both eyes, Touch controllers, hand tracking, and passthrough through `XR_FB_passthrough`. A panel app opens an immersive session over its panel and returns to it. |
| Android XR                                | The same OpenXR backend, through the system runtime. There is no `--platform` for Android XR yet; the Android build carries the loader.                                                               |
| Windows, Linux                            | OpenXR with the `openxr` feature, in a host that registers runtime plugins itself; the generated Windows and Linux apps don't register plugins yet.                                                   |
| Web                                       | WebXR's `immersive-vr` and `immersive-ar`, with controllers and hands, in the Meta Quest browser and other WebXR browsers. Start the session from a click.                                            |
| iOS, macOS                                | No headset support. Apple Vision Pro is not covered.                                                                                                                                                  |
| The terminal, HarmonyOS, Wear OS, watchOS | Not supported.                                                                                                                                                                                        |
