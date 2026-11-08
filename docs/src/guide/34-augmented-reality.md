# Augmented reality on phones

<Callout kind="warning" title="Experimental">

The `xr` plugin is new. Its protocol, its frame stream, recorded playback, and drawing a scene over a camera picture are tested; the ARKit, ARCore, Camera2, and WebXR backends build for their platforms but have not yet been checked on a device. Expect to find rough edges, and report them.

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
| `depth`    | Whether the frame has the room's depth, measured or estimated, so things hide behind real furniture.                          |
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

## Draw over the camera

A [3D scene](33-3d-scenes.md) whose backdrop is the camera draws over the live camera picture while a session runs. Ask the session for the picture, and for depth so real things hide what is behind them:

```rust
use leptoskit::prelude::*;
use leptoskit::scene3d::Backdrop;
use leptoskit::scene3d::Op;
use leptoskit::scene3d::SceneId;
use leptoskit::scene3d::client::Scene;
use leptoskit_xr::Features;
use leptoskit_xr::Mode;
use leptoskit_xr::SessionConfig;

const ROOM: SceneId = SceneId(1);

#[component]
fn Hunt() -> Element {
	view! {
		<Scene3d scene=ROOM aria_label="Your room, with the creature in it" class="w-full h-full" />
	}
}

fn hunt_config(mode: Mode) -> SessionConfig {
	SessionConfig::new(mode).prefer(Features::CAMERA_IMAGE | Features::DEPTH | Features::PLANES)
}

async fn show_the_room() -> Result<(), leptoskit::scene3d::client::ClientError> {
	let scene = Scene::new(ROOM);
	scene.push(Op::SetBackdrop(Backdrop::Camera));
	scene.flush().await
}
```

While the session runs, the native app draws the picture turned upright for the screen and cropped to fill the view, and draws the scene from where the phone is: the session's own view replaces the scene's camera, so place models in the session's world coordinates (the same ones `viewer`, `planes`, and hit tests use). Where the phone measures depth, models behind a real wall or table are hidden by it. When the session stops, the scene draws transparent behind its models again.

On Android phones without Google Play Services for AR, an `Orientation` session shows the back camera too, and its view has the camera's field of view, so a creature keeps the picture's scale as the user turns. Ask for the camera permission before you start it: without the permission the session starts without the picture, unless it requires `CAMERA_IMAGE`. On Android the view moves below the app's other views while it shows the camera, so a heads-up display laid over it stays on top.

In the browser, the WebXR session draws the camera itself; a `Scene3d` doesn't draw over it yet.

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
| Android                 | ARCore: planes, the Depth API on supported phones, ambient light, hit tests, anchors. Phones without Google Play Services for AR use the rotation sensor, with the back camera behind the scene. ARCore has no plane labels, so planes are `Unknown`.    |
| Web                     | WebXR's `immersive-ar` in Chrome on Android: hit tests, anchors, and where the browser offers them, planes and depth. Start the session from a tap. Without WebXR AR, `deviceorientation` turns the view (Safari asks for motion permission from a tap). |
| macOS, Windows, Linux   | No AR. `capabilities` reports nothing; OpenXR headsets on Windows and Linux are covered in [VR and AR on headsets](35-headsets.md).                                                                                                                      |
| The terminal, HarmonyOS | No AR: the plugin isn't registered and `Xr::new` fails with `Unavailable`.                                                                                                                                                                               |
| Wear OS and watchOS     | Not supported.                                                                                                                                                                                                                                           |

Phones don't know where the floor is when a session starts: `LocalFloor` is reported as `World`, whose origin is where the phone was when tracking began. Find the floor from the planes.

## Depth on phones that measure none

Hiding a creature behind the sofa needs the room's depth, and many phones don't measure it: the Solana Seeker has no ARCore, many ARCore phones lack the Depth API, and only Pro iPhones and iPads have LiDAR. Without help, `frame.depth` is `false` on those phones, and hit tests reach planes but not the furniture standing on them.

The `xr` plugin can estimate the depth from the camera picture instead. A small neural network, Depth Anything V2 Small fine-tuned for rooms (the `leptoskit_depth` crate), looks at one picture and says how far away each part of it is, in metres, on the phone's CPU: about a quarter of a second a picture on a laptop-class core, and slower on a phone, so it reads about one picture a second, off the thread that draws. Nothing leaves the phone.

It needs the model file, 27 MB under Apache-2.0, which your app ships rather than leptoskit. Fetch it once into your `assets/` folder, which `leptoskit build` bundles into the iOS app and the Android APK, and keep its licence beside it:

```sh
mkdir -p assets/depth
curl -L -o assets/depth/depth_anything_v2_metric_indoor_small.onnx \
  https://github.com/pina-rs/reapers/releases/download/depth-model-v1/depth_anything_v2_metric_indoor_small.onnx
echo "b81c4fae8985bd73d5026ffc1ebf545ff01d73fbf4d534a06c811abfcadb6dbd  assets/depth/depth_anything_v2_metric_indoor_small.onnx" | shasum -a 256 -c
curl -L -o assets/depth/LICENSE.txt https://www.apache.org/licenses/LICENSE-2.0.txt
```

Then name it in `leptoskit.toml`, relative to the `assets/` folder:

```toml
[native.plugin-options.xr]
max-frame-rate = 30
depth-model = "depth/depth_anything_v2_metric_indoor_small.onnx"
```

`leptoskit build` then builds the plugin with the model's runtime (its `depth-estimation` feature), and the native app loads the file in the background when it starts, in half a second to a second and a half, and checks that it is exactly the file above. A missing or different file is written to the device's log (logcat's `leptoskit-xr` tag on Android, Xcode's console on iOS) and depth stays off; the app runs as before.

The model runs in Rust compiled for your app, and a debug build is not optimised: it takes nearly twenty seconds a picture instead of a quarter of one. Optimise its crates in your workspace's `Cargo.toml`, so `leptoskit dev` builds stay usable:

```toml
[profile.dev.package.tract-core]
opt-level = 3

[profile.dev.package.tract-data]
opt-level = 3

[profile.dev.package.tract-linalg]
opt-level = 3
```

Nothing changes in your app's code. Once the model has loaded, a session that asks for `Features::DEPTH` on such a phone gets it, as `prefer` or `require`: `frame.depth` turns `true` when the first picture has been read, a `Scene3d` over the camera hides models behind real things, and `DEPTH` hit tests reach the room. Phones that measure depth keep their own; estimation never replaces it. A session started before the model loaded doesn't have estimated depth; start it again.

What to expect from it:

- **Its metres are approximate.** Against rooms measured with a depth sensor, a whole picture read between 13% short and 14% long. Nearer and farther stay in order, so a creature still hides behind the right things; measure against the floor where an exact size matters.
- **It sees a portrait crop.** The model reads a 9:16 upright picture from the middle of the camera's; outside it there is no depth, so a phone held in landscape gets depth only in a strip down the middle, and a hit test outside it finds nothing.
- **It is a moment old.** Each reading is the room as it was when its picture was taken, up to a second ago. Hit tests cast against it from where the phone was then, so a ray from where the phone is now still meets the sofa where it stands. Occlusion turns the reading to the way the phone faces now, which is close but not exact while the phone turns quickly.
- **It can't see in the dark, through glass, or into mirrors**, and nor can a depth sensor.

## Test without a phone

A recording plays back frames exactly as they were captured, so a test, a simulator, or a desktop build can run the same session over and over. Record one from a live session with `leptoskit_xr::host::Recorder`, save `recording.to_bytes()`, and point the plugin at it:

```toml
[native.plugin-options.xr]
replay = "walk-through-the-living-room.lkxr"
replay-loop = true
```

A relative path is read from the app's data directory; the iOS simulator can also read an absolute path on your Mac. Playback reads no clock of its own, so the same updates give the same frames on every run.
