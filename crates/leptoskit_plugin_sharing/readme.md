<h1 align="center">leptoskit_plugin_sharing</h1>

<p align="center">
  <strong>leptoskit_plugin_sharing</strong> is the leptoskit plugin for sharing text, web URLs and files through the user-selected native share UI, reporting the chosen destination where the platform says.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_sharing"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_sharing"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_sharing"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_sharing.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_sharing"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_sharing/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

A port of Flutter's [`share_plus`](https://github.com/fluttercommunity/plus_plugins/tree/main/packages/share_plus/share_plus). A share carries one of:

- text (up to 64 KiB of UTF-8) or an explicitly validated HTTP(S) URL (up to 8192 bytes), never both;
- one to 32 files, optionally with text.

Any share can add an email subject (`with_subject`, up to 1 KiB), a sheet title (`with_title`, up to 1 KiB), a popover anchor (`with_anchor`) and iOS activities to leave out (`excluding`). The user chooses the destination through iOS `UIActivityViewController`, macOS `NSSharingServicePicker`, Android's system chooser, or the browser Web Share API. No direct recipient or automatic sending API is exposed.

### Files

`File::app("exports/report.pdf")` names a regular file inside the app's files directory, the root the `filesystem` plugin reads and writes (`<Application Support>/files` on iOS, `<Application Support>/<bundle id>/files` on macOS, `filesDir/files` on Android). Paths follow the `filesystem` plugin's rules: relative, no `.`/`..` or empty segments, no alternate separators or reserved names. The native host checks them again, opens the file without following a final symlink, and requires the opened file to resolve inside that directory (macOS/iOS `F_GETPATH`, Android `/proc/self/fd`), so neither a symlink nor a swapped directory exposes anything else. `File::bytes("photo.png", bytes)` shares in-memory contents; all in-memory files of one share hold at most 4 MiB. Each file can carry a MIME type (`with_mime`, `type/subtype`, `type/*` or `*/*`, no parameters) and a display name (`with_name`, one segment up to 255 bytes).

Natively, every file is copied into a private cache directory (`Caches[/<bundle id>]/leptoskit-sharing`, Android `cacheDir/leptoskit-sharing`) under its display name, off the UI thread. Copies are limited to 512 MiB per file and 1 GiB per share. The cache is cleared at the start of the next file share rather than when the sheet closes, because a destination such as a mail draft may read the file later; the OS may also purge it.

On Android the plugin ships `SharingFileProvider`, declared through `android-file-providers` metadata: the generated manifest gets an unexported provider with authority `<application id>.leptoskit.sharing` serving only `cacheDir/leptoskit-sharing`. The share intent carries the copies as `content://` URIs in `EXTRA_STREAM` and `ClipData` with `FLAG_GRANT_READ_URI_PERMISSION`, so only the chosen app gets read access to only those copies. One file uses `ACTION_SEND` with its MIME type (given, or guessed from the extension, else `application/octet-stream`); several use `ACTION_SEND_MULTIPLE` with the narrowest common type. The plugin adds `androidx.core:core:1.16.0`.

### Result

`share` returns a `ShareResult`:

| Platform | `Success(raw)`                                                                    | `Dismissed`                                  | `Unavailable`                        |
| -------- | --------------------------------------------------------------------------------- | -------------------------------------------- | ------------------------------------ |
| iOS      | The activity type, such as `com.apple.UIKit.activity.Mail`                        | The sheet closed without completing          | An activity completed without a type |
| macOS    | The chosen service's title (AppKit exposes no identifier)                         | The picker or the service's UI was closed    | Never                                |
| Android  | The chosen component, `package/class`, from the chooser's `IntentSender` callback | The chooser reported no component            | Never                                |
| Browser  | Never                                                                             | `navigator.share` rejected with `AbortError` | `navigator.share` resolved           |

On Android, chooser actions that start no app (such as Copy) report `Dismissed`. A result never confirms delivery: a chosen app can still discard the content. Platform failures remain errors. Caller cancellation cannot retract content already handed to another service; macOS retains its busy slot until the chosen service finishes, unless the whole plugin closes.

### Platform details

- **Registration:** enable `guest` for native bridge calls and select `sharing` in native configuration. Apple registration injects the owning `NativePluginContext`; Android requires the generated activity's `NativeActivityResultHost`. No permissions or entitlements are added; Android gains the plugin's `FileProvider`. Custom hosts must provide the same presentation capabilities and declare the provider.
- **iPad and macOS anchor:** `Anchor::new(x, y, width, height)` is a rectangle in points within the app's root view, origin top left; values must be finite, non-negative and at most 1,000,000. The host refuses an origin outside the view and clips the size to it. Without one, iPad and macOS point at the middle of the view. iPhone presents a sheet and never configures the popover.
- **Subject and title:** iOS supplies the subject through `UIActivityItemSource` and the title as the preview's `LPLinkMetadata` title. macOS sets the chosen service's subject, using the title when there is no subject. Android sets `EXTRA_SUBJECT`, `EXTRA_TITLE` and the chooser title. Browsers get `title` (the subject when there is no title).
- **Excluded activities:** iOS only; macOS's picker and other platforms ignore them.
- **Browser:** apps call `web::share(&content)?.await?` directly inside a trusted user action; the call starts synchronously to preserve activation. HTTPS and Web Share support are required; `web::is_available()` checks these without opening UI. Files must be in-memory bytes (Web Share Level 2); the browser must offer `navigator.canShare` and accept the data, or the call returns `Unavailable` before opening UI. An app-directory file is also `Unavailable` in the browser: read it and share its bytes. Unlike `share_plus`, there is no download or `mailto:` fallback. Dropping the future cannot close browser UI; busy admission lasts until the browser promise settles.
- **Windows and Linux:** no native share sheet is implemented (`Unavailable`). Windows' `DataTransferManager` needs window-owned COM interop the generated host does not provide yet, and Linux has no standard share portal; `share_plus`'s `mailto:` fallback is not reproduced. Supported browsers on those platforms may expose Web Share.

### Wire format

Method 0 takes, little-endian: version `1`; length-prefixed text, URL, subject and title (length 0 when absent); an anchor flag and four `f32` bit patterns; a count of excluded activity codes (0–19, in `share_plus`'s `CupertinoActivityType` order); a count of files, each a source (0 app path, 1 bytes), its length-prefixed path or bytes, MIME type and display name. The reply is version `1`, a status (0 success, 1 dismissed, 2 unavailable) and a length-prefixed raw identifier present exactly for success (up to 1024 bytes, no controls). Rust, Swift and Kotlin decoders validate every bound, code and length and refuse trailing data, older protocol versions and unknown versions before presenting UI; the three test suites check the same golden bytes. The previous protocol's request (kind, value) and empty reply are no longer accepted.

One presentation per plugin is admitted, including while files are being copied, and the owning host rejects competing Android external activities. Apple uses the explicit app controller; it never searches for a global window. Native URL parsers can reject a stricter subset than the Rust constructor; for example, Android rejects percent-encoded hostnames and abbreviated IPv4 addresses. Use ordinary ASCII or punycode hostnames and full IPv4 addresses for portable URLs.

`share_plus`'s Android-only preview thumbnail (unfinished upstream), arbitrary filesystem paths, custom URL schemes, recipient addresses and incoming share extensions are not part of this port. All automated tests use synthetic data and mocked completion; they never open a share destination or send content.

References: [`share_plus`](https://pub.dev/packages/share_plus), [Expo Sharing](https://docs.expo.dev/versions/latest/sdk/sharing/), [Android sharing](https://developer.android.com/training/sharing/send), [Android FileProvider](https://developer.android.com/reference/androidx/core/content/FileProvider), [Apple activity UI](https://developer.apple.com/documentation/uikit/uiactivityviewcontroller), [Apple sharing picker](https://developer.apple.com/documentation/appkit/nssharingservicepicker), [Web Share](https://developer.mozilla.org/en-US/docs/Web/API/Navigator/share).

## How it fits

<!-- {=p_sharingSummary} -->

Add `leptoskit_plugin_sharing` as a direct dependency and select the `sharing` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Linux hosts register it where it has a Linux Rust implementation; generated Windows and terminal hosts currently omit runtime plugins, so use explicit registration in a custom host there. The app supplies text, a web URL, or files from its own files directory or memory; the user picks the destination, and the reply says which one where the platform reports it. It never confirms delivery.

<!-- {/p_sharingSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_sharing::{share, Anchor, Content, File, ShareResult};

let content = Content::files([
    File::app("exports/report.pdf")?.with_mime("application/pdf")?,
    File::bytes("chart.png", png_bytes)?.with_mime("image/png")?,
])?
.with_text("This week's report")?
.with_subject("Weekly report")?
.with_anchor(Anchor::new(24.0, 80.0, 44.0, 44.0)?);

match share(&content).await? {
    ShareResult::Success(destination) => log(&destination),
    ShareResult::Dismissed | ShareResult::Unavailable => {}
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_sharing)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
