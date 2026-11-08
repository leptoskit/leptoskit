---
"release": minor
---

# Bundled images on every platform, and launch screens

`Image("asset://portraits/ada.webp")` shows `assets/portraits/ada.webp` from your project on the web, iOS, Android, macOS, Windows, Linux, and HarmonyOS (experimental). The browser loads it from `/assets/`, where the web build already serves the folder, and `leptoskit build` now copies `assets/` into every native app: the iOS app bundle, the APK's assets, `Contents/Resources` on macOS, beside the Windows and Linux executable, and HarmonyOS's `rawfile`. Only changed files are copied again, deleted ones leave the app, and hidden files stay behind. Before, the native apps bundled the folder only when they showed 3D scenes. PNG, JPEG, and WebP decode natively; Linux needs the system's WebP loader for WebP. Assets ship with the store build, not with code push updates.

A missing asset stops a debug build with an error that names the source and where it looked, and a release build logs it and leaves the image empty. `leptoskit_native::asset` parses sources the same way on every host and refuses paths that leave the folder.

The audio plugin's `Source::Asset` reads the same copy, so sounds in `assets/` now play on native apps without copying them into the host project. On Android it reads `assets/<path>` inside the APK's assets, where the build puts them; a file you placed at the root of `android/app/src/main/assets/` by hand moves into an `assets/` folder there, or into the project's `assets/`.

`[app.launch_screen]` gives iOS and Android a launch screen: a `background` colour, an optional `background_dark`, and an optional square `image` (PNG or SVG, read like `app.icon`) drawn at `image_size` points or dp. iOS gets a colour set and image set in its asset catalog, named by `UILaunchScreen` in the generated `Info.plist`. Android gets a launch theme for the launcher activity, extending the application's own theme, with the splash screen attributes on Android 12 and later and a window background before; once the first screen draws, the generated app puts the theme's own background back.
