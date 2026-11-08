# Augmented reality on phones

<Callout kind="warning" title="Experimental">

The `xr` plugin is new. Its protocol, its frame stream, and recorded playback are tested; the ARKit, ARCore, and WebXR backends build for their platforms but have not yet been checked on a device. Expect to find rough edges, and report them.

</Callout>

Some apps put things in the room. A game hides a creature behind the sofa and lets it walk across the real floor; a furniture app shows a chair where the chair would go. For that the app needs to know where the phone is and which way it points, where the floor and walls are, and how far away everything is, many times a second. The `xr` plugin, from the `leptoskit_xr` crate, asks the phone's AR platform for all of it and hands your app a stream of frames.

## What a frame tells you

Each frame says where the viewer is, how the screen sees the world, and what the phone has found so far:

| Field      | What it holds                                                                                                                 |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `viewer`   | The phone's position and rotation, in metres, with +Y up.                                                                     |
| `views`    | One view per screen (two on a headset): its pose and its field of view, for drawing.                                          |
| `planes`   | Flat surfaces found so far: floors, tables, walls, with their outlines. On recent iPhones each is labelled `Floor`, `Wall`, … |
| `anchors`  | Points you pinned, where they are now: they move as the phone corrects its map.                                               |
| `light`    | How bright the room is, so a creature isn't lit like a studio in a dark hallway.                                              |
| `depth`    | Whether the phone measured the room's depth this frame, so things hide behind real furniture.                                 |
| `tracking` | Whether tracking is `Normal`, `Limited` (and why: too dark, moving too fast), or lost.                                        |

The camera picture and the depth picture never reach your app's code. The native app draws the camera behind your 3D scene and hides the parts of the scene that are behind real things; your code decides where things are.

## Start a session

Add the crate, select the plugin, and the `permissions` plugin for the camera:

```toml
# Cargo.toml
[dependencies]
leptoskit_xr = { version = "0.3", features = ["guest"] }
leptoskit_plugin_permissions = { version = "0.3", features = ["guest"] }

# leptoskit.toml
[native]
plugins = ["xr", "permissions"]

[native.plugin-options.xr]
max-frame-rate = 30
```

`max-frame-rate` caps how many frames a second your code receives. Tracking runs at the camera's own rate either way; your code always gets the newest frame, so a slow frame never builds a backlog.

iOS needs a reason for the camera in the app's `Info.plist`, under `NSCameraUsageDescription`, such as "Reapers shows creatures in your room through the camera." The build stops and asks for it if it is missing.

Then ask what the phone can do, and start the best mode it has. Phones without AR still turn the view as the user turns, so the app works everywhere:

```rust
use leptoskit_xr::Availability;
use leptoskit_xr::Features;
use leptoskit_xr::Mode;
use leptoskit_xr::SessionConfig;
use leptoskit_xr::Xr;

async fn start_hunt() -> Result<leptoskit_xr::Session, leptoskit_xr::Error> {
	let xr = Xr::new()?;
	let mode = xr
		.capabilities()
		.await?
		.first_of(&[Mode::PhoneAr, Mode::Orientation])
		.ok_or(leptoskit_xr::Error::Unavailable)?;

	if xr.prepare(mode, true).await? == Availability::NeedsPermission {
		ask_for_the_camera().await?;
	}

	xr.start(
		SessionConfig::new(mode)
			.require(Features::HIT_TEST)
			.prefer(Features::PLANES | Features::DEPTH | Features::LIGHT_ESTIMATION),
	)
	.await
}
```

`prepare` answers `Ready`, `NeedsPermission` (ask with the `permissions` plugin, then try again), `NeedsInstall` (Android without Google Play Services for AR: passing `true` opens the store), `Checking`, or `Unsupported`. `require` makes a session fail with `Unavailable` when the phone lacks a feature; `prefer` turns a feature on where the phone has it, and `session.info().features` says which ones you got.

## Follow the frames

Read frames in a loop and keep what you need in signals. Planes arrive complete in every frame you receive, but they are only sent over when they change, so a room full of planes costs nothing per frame:

```rust
use leptos::prelude::*;

let creature_floor = RwSignal::new(None);

leptos::task::spawn_local(async move {
	let mut session = start_hunt().await?;

	while let Ok(frame) = session.next_frame().await {
		let floor = frame
			.planes
			.iter()
			.find(|plane| plane.label == leptoskit_xr::PlaneLabel::Floor);
		creature_floor.set(floor.map(|plane| plane.pose));
	}

	Ok::<_, leptoskit_xr::Error>(())
});
```

## Put things where the user taps

A hit test casts a ray through a point on the screen and returns where it meets the real world, nearest first, with a pose whose +Y is the surface's normal: a ready-made place for something to stand. Pin it with an anchor so it stays put while the phone refines its map:

```rust
use leptoskit_xr::HitTestRequest;
use leptoskit_xr::Session;

/// Pins a trap where the user tapped, `x` and `y` from 0 to 1 across and
/// down the screen. Returns the anchor, or `None` when the tap hit nothing.
async fn place_trap(session: &Session, x: f32, y: f32) -> Result<Option<u64>, leptoskit_xr::Error> {
	let hits = session.hit_test(HitTestRequest::screen(x, y)).await?;

	match hits.first() {
		Some(hit) => Ok(Some(session.add_anchor(hit.pose, hit.plane).await?)),
		None => Ok(None),
	}
}
```

Call it from the screen's press handler with the touch's position divided by the screen's size. The anchor's pose then arrives in every frame's `anchors`, and the creature is drawn there.

Hit tests can aim at `PLANES` (inside their outlines), `INFINITE_PLANES` (past their edges), `DEPTH` (any surface, flat or not, where the phone measures depth), and `ESTIMATED` (a guess before a plane is found). A ray from a controller or from code works the same way with `HitTestRequest::ray`.

## On each platform

| Platform                | What it does                                                                                                                                                                                                                                             |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| iOS                     | ARKit world tracking: horizontal and vertical planes with labels, raycasts, anchors, light, and LiDAR depth on iPhone Pro and iPad Pro. Orientation from Core Motion elsewhere. The simulator has no AR; play a recording there (below).                 |
| Android                 | ARCore: planes, the Depth API on supported phones, ambient light, hit tests, anchors. Phones without Google Play Services for AR use the rotation sensor. ARCore has no plane labels, so planes are `Unknown`.                                           |
| Web                     | WebXR's `immersive-ar` in Chrome on Android: hit tests, anchors, and where the browser offers them, planes and depth. Start the session from a tap. Without WebXR AR, `deviceorientation` turns the view (Safari asks for motion permission from a tap). |
| macOS, Windows, Linux   | No AR. `capabilities` reports nothing; OpenXR headsets on Windows and Linux are covered in [VR and AR on headsets](35-headsets.md).                                                                                                                      |
| The terminal, HarmonyOS | No AR: the plugin isn't registered and `Xr::new` fails with `Unavailable`.                                                                                                                                                                               |
| Wear OS and watchOS     | Not supported.                                                                                                                                                                                                                                           |

Phones don't know where the floor is when a session starts: `LocalFloor` is reported as `World`, whose origin is where the phone was when tracking began. Find the floor from the planes.

## Test without a phone

A recording plays back frames exactly as they were captured, so a test, a simulator, or a desktop build can run the same session over and over. Record one from a live session with `leptoskit_xr::host::Recorder`, save `recording.to_bytes()`, and point the plugin at it:

```toml
[native.plugin-options.xr]
replay = "walk-through-the-living-room.lkxr"
replay-loop = true
```

A relative path is read from the app's data directory; the iOS simulator can also read an absolute path on your Mac. Playback reads no clock of its own, so the same updates give the same frames on every run.
