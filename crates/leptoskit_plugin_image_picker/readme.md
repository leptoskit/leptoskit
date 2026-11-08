<h1 align="center">leptoskit_plugin_image_picker</h1>

<p align="center">
  <strong>leptoskit_plugin_image_picker</strong> is the leptoskit plugin for picking one photo-library image or video into an app-owned temporary URI with cancellation and size bounds.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_image_picker"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_image_picker"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_image_picker"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_image_picker.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_image_picker"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_image_picker/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Presents the system photo library (or a browser file input), copies one selected image or video into app-owned temporary storage under a **16 MiB** default ceiling, and returns a temporary URI with MIME type, pixel size when known, and byte size. Cancellation is an error. Only one picker may run at a time (`Busy`). This plugin does **not** replace `document_picker`, and it does **not** capture from the camera — that belongs to a later `camera` plugin.

Add the crate to the native host and select `"image-picker"` in `native.plugins`. Apple registration supplies the owning `NativePluginContext`; Android uses the generated activity's `NativeActivityResultHost`.

Guest applications enable the `guest` feature. Browser applications call `web::pick(options)` directly from a trusted user action.

| Platform            | Picker                                                           | Result                                                              |
| ------------------- | ---------------------------------------------------------------- | ------------------------------------------------------------------- |
| iOS / macOS         | `PHPickerViewController` (no photo-library purpose string)       | App temp `file:` URI; swipe-dismiss and empty selection → Cancelled |
| Android             | `ACTION_GET_CONTENT` through the activity-result broker          | App-cache `file:` URI; `RESULT_CANCELED` → Cancelled                |
| Browser             | Hidden `<input type=file accept=image/*,video/*>` + `showPicker` | `blob:` object URL the page must revoke; cancel event → Cancelled   |
| Windows/Linux guest | Unavailable (no Auto registration)                               | Direct desktop adapters are not part of this MVP                    |

Photo-library purpose strings (`NSPhotoLibraryUsageDescription` and friends) remain **app-owned** if you later adopt limited-library or pre-PHPicker APIs. This MVP's PHPicker path does not require them. Multi-select, editing UI, crop, and camera capture remain out of scope.

Sources: [Expo ImagePicker](https://docs.expo.dev/versions/latest/sdk/imagepicker/), [PHPicker](https://developer.apple.com/documentation/photokit/phpickerviewcontroller), [Android photo picker](https://developer.android.com/training/data-storage/shared/photopicker).

## How it fits

<!-- {=p_image_pickerSummary} -->

Add `leptoskit_plugin_image_picker` as a direct dependency and select the `image-picker` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. The user picks one library image or video; the app receives a temporary URI and metadata under a size limit, never a persistent gallery grant. Camera capture is deferred to a later `camera` plugin.

<!-- {/p_image_pickerSummary} -->

## Use it

```rust,ignore
let asset = leptoskit_plugin_image_picker::pick(
    leptoskit_plugin_image_picker::Options::new(
        leptoskit_plugin_image_picker::MediaKind::Images,
        4 * 1024 * 1024,
    )?,
).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_image_picker)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
