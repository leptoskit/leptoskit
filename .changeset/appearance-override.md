---
"release": minor
---

# Let people choose light or dark

`dark:` utilities followed only the device's appearance, so an app could not offer "Light", "Dark", and "Match device". `leptoskit::appearance::set(Appearance::Light)` (or `Dark`, or `System` to follow the device again) now switches every `dark:` utility at once, without a reload, and `appearance::get()` reads the choice reactively.

On the web, the choice marks `<html>` with `data-lk-appearance` and sets its `color-scheme`; the generated stylesheet applies `dark:` rules under `prefers-color-scheme: dark` unless the root says `light`, and always when it says `dark`. In a native app, the root of the next render carries the choice (`NativeProps::appearance`), and `NativeSurface` resolves `dark:` against it and passes the result to the backend's `set_dark_mode`, so native controls match. Hosts can also call `NativeSurface::set_appearance`, and read `NativeSurface::appearance` and `NativeSurface::is_dark`. Older shells ignore the choice and keep following the device.

Persisting the choice is the app's job: store `Appearance::as_str()` and call `appearance::set(stored.parse()?)` before the first render.
