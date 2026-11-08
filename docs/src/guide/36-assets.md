# Images and other assets

An app ships pictures: portraits, illustrations, a logo on the sign-in screen. The browser loads a picture from a URL, but an iOS or Android app has no server to ask: the picture has to be inside the app, and each platform keeps its files somewhere different. Put the files in your project's `assets/` folder and name them with `asset://`, and the same `Image` shows them on every platform.

## Show a bundled picture

With a portrait at `assets/portraits/ada.webp` in your project:

```rust
use leptoskit::prelude::*;

#[component]
pub fn Portrait() -> impl IntoView {
	view! {
		<Image source="asset://portraits/ada.webp" class="w-24 h-24 rounded-lg" accessibility_label="Ada Lovelace" />
	}
}
```

This works on the web, iOS, Android, macOS, Windows, Linux, and HarmonyOS (experimental). The path after `asset://` is the file's path inside `assets/`:

| Platform          | Where the picture is read from                                                  |
| ----------------- | ------------------------------------------------------------------------------- |
| Web               | `/assets/portraits/ada.webp`, where the web build serves the folder             |
| iOS               | `assets/portraits/ada.webp` inside the app bundle                               |
| Android           | `assets/portraits/ada.webp` inside the APK's assets, through the `AssetManager` |
| macOS             | `Contents/Resources/assets/portraits/ada.webp` inside the `.app`                |
| Windows and Linux | `assets/portraits/ada.webp` beside the executable                               |
| HarmonyOS         | the HAP's `rawfile/assets/portraits/ada.webp`                                   |
| The terminal      | nothing: the terminal draws its image placeholder                               |

When the path is built at run time, `leptoskit::native::asset::asset("portraits/ada.webp")` makes the source for you.

## Formats

PNG, JPEG, and WebP work everywhere. Each platform decodes them itself: iOS 14 and later, macOS 11 and later, and every Android version leptoskit supports read WebP, and the Windows app decodes it with its own decoder. On Linux, GTK reads images through the system's image loaders, so WebP needs the WebP loader for GdkPixbuf (`webp-pixbuf-loader`), which most desktops install.

Pictures are decoded when they are first shown, on the main thread, so keep them close to the size they are drawn at.

## How the folder reaches each app

`leptoskit build` copies `assets/` into every native app it builds, and `leptoskit dev` does the same when it builds a native app as it starts. While `leptoskit dev` runs, it reloads your code, not the folder, so restart it to see a picture you've just added on a device. Only what changed is copied again; a file you delete leaves the app at the next build. Hidden files such as `.DS_Store` stay behind. The web build serves the same folder at `/assets/`, so a page can also link a file directly, as the starter does with `/assets/app.css`.

The same folder holds your sounds for the [audio plugin](34-audio.md) and your models for [3D scenes](33-3d-scenes.md). Everything in it ships with every app, so keep files the app doesn't use elsewhere.

## Assets and code push

Assets ship with the store build. A [code push](05-code-push.md) update carries your app's code, not the `assets/` folder, so an update that shows a picture the installed build doesn't have can't show it. Add new pictures in a store release first, and send updates that use them after it.

## Paths

The path after `asset://` is relative to `assets/`, separated by `/`, and must stay inside the folder: an empty segment, `.`, or `..` is refused. Paths are case-sensitive, as they are on Android, Linux, and the web, so match the file's name exactly.

## A missing picture

A picture that isn't in the app is a mistake you should see at once, so a debug build stops on it. The iOS, macOS, and Android apps show an error that names the source and where it looked; HarmonyOS shows the same error in every build. A release build logs the error and leaves the image empty, so one missing picture never takes the screen down. In the browser, a missing picture is a failed request in the developer tools and the browser's broken-image mark.

HarmonyOS projects made before `asset://` keep their own `entry/src/main/ets/pages/Index.ets`. Copy the image handling from a new project's, or recreate the host, to show bundled pictures there.
