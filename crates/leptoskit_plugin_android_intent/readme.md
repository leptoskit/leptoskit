<h1 align="center">leptoskit_plugin_android_intent</h1>

<p align="center">
  <strong>leptoskit_plugin_android_intent</strong> is the leptoskit plugin for launching, broadcasting and resolving arbitrary Android intents with typed extras.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#security"><strong>Security</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_android_intent"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_android_intent"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_android_intent"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_android_intent.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_android_intent"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_android_intent/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Some Android features have no cross-platform API: opening a specific settings page, handing a link to one app's activity with extras it expects, asking another app's service to start, or sending a broadcast your own receiver listens for. On Android all of these are intents: an action, categories, a data URI, a MIME type, a package or component, flags, and a bundle of extras. `leptoskit_plugin_linking` only opens safe web, email, phone and SMS links; this plugin covers the rest, as Flutter's [`android_intent_plus`](https://pub.dev/packages/android_intent_plus) does.

It mirrors `android_intent_plus`'s API:

| `android_intent_plus`          | `guest::AndroidIntent`          | Android call                                          |
| ------------------------------ | ------------------------------- | ----------------------------------------------------- |
| `launch()`                     | `launch(&intent)`               | `startActivity`                                       |
| `launchChooser(title)`         | `launch_chooser(&intent, ..)`   | `startActivity(Intent.createChooser(intent, title))`  |
| `sendBroadcast()`              | `send_broadcast(&intent)`       | `sendBroadcast`                                       |
| `sendService()`                | `send_service(&intent)`         | `startService` (the intent must name a package)       |
| `canResolveActivity()`         | `can_resolve_activity(&intent)` | `resolveActivity(…, MATCH_DEFAULT_ONLY) != null`      |
| `getResolvedActivity()`        | `resolved_activity(&intent)`    | the package, activity class and label it resolves to  |
| `AndroidIntent.parseAndLaunch` | `parse_and_launch(uri)`         | `Intent.parseUri(uri, URI_INTENT_SCHEME)`, then start |

`Intent` carries `action`, `category` (several allowed), `data`, `mime_type`, `package`, `component`, `flag` and `extra`. Extras cover upstream's `arguments` (`bool`, `i32`, `i64`, `f64`, `String`, nested `Extras` bundles, byte arrays, `ArrayList<Integer>` and `ArrayList<String>`) and its `arrayArguments` (boolean, int, long, double and string arrays), through the `Extra` enum and `From` conversions. Upstream's action aliases are constants: `action_view` is `action::VIEW`, `settings` is `action::SETTINGS`, and so on; its `Flag` class is the `flag` module.

**Platforms.** Only Android phones and tablets register the plugin. On the web, iOS, macOS, Windows, Linux, the terminal, HarmonyOS, Wear OS and watchOS, `AndroidIntent::new()` returns `Unavailable`, so call it once and hide Android-only features when it fails. Hosts without Android can register `Unsupported`, which validates each request like Android and then answers `Unavailable`.

**Errors.** `Unavailable` also means no activity or service handles the intent (Android's `ActivityNotFoundException`, or `startService` returning no component or refusing a background start). `PermissionDenied` means Android refused with a `SecurityException`, or this plugin's policy refused the request. `TooLarge` means a bound was exceeded and `InvalidArgument` means a malformed field. Success means Android accepted the request; it does not mean the activity did anything, and Android 10+ silently drops activity starts from the background.

Behaviour that differs from upstream on purpose:

- `launch` never clears the package of an intent that does not resolve. Upstream retries such intents without their package, which can hand extras meant for one app to any app that matches.
- `parse_and_launch` treats the URI the way a browser does: it refuses a `component=` or `SEL;` selector, refuses URI grant flags and `file:` data, and adds `CATEGORY_BROWSABLE`. The link has to name a scheme, and the activity it resolves to has to declare that category on the filter that matched. An action-only URI such as `intent:#Intent;action=android.settings.SETTINGS;end` is `Unavailable`: API 35 marks that Settings activity browsable, so starting it would open Settings. Use `launch` with `Intent::component` for explicit components. `S.browser_fallback_url` is not followed; open it yourself with `leptoskit_plugin_linking` when the call returns `Unavailable`.
- MIME types must be lowercase, because Android matches them case-sensitively.

### Android 11 package visibility

On Android 11 and later, `can_resolve_activity` and `resolved_activity` only see apps your manifest makes visible, and answer `false` or `None` for the rest; `launch` is not affected. Declare what you query in your app's `android/app/src/main/AndroidManifest.xml`, which leptoskit merges and never overwrites:

```xml
<queries>
  <intent>
    <action android:name="android.intent.action.VIEW" />
    <data android:scheme="https" />
  </intent>
  <package android:name="com.google.android.apps.maps" />
</queries>
```

Plugin metadata can add `<queries>` entries (`android-query-actions` and `android-query-intents`), but only fixed ones every app would share. This plugin adds none, because which apps your app may see is your decision and Google Play reviews broad visibility. Some packages are [visible automatically](https://developer.android.com/training/package-visibility/automatic), including your own app.

## How it fits

<!-- {=p_android_intentSummary} -->

Add `leptoskit_plugin_android_intent` as a direct dependency and select the `android_intent` plugin in `native.plugins`; generated Android hosts discover it from Cargo metadata. Guests enable the `guest` feature and reuse `guest::AndroidIntent` rather than resolving the plugin on every call. It launches, broadcasts and resolves arbitrary Android intents with typed extras, the way Flutter's `android_intent_plus` does. Only Android registers it: on the web, iOS, macOS, Windows, Linux, the terminal, HarmonyOS, Wear OS and watchOS, `guest::AndroidIntent::new` returns `Unavailable`. Guest code can change over the air, so the plugin refuses URI-permission grant flags, `file:` URIs and parcelable extras.

<!-- {/p_android_intentSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_android_intent::guest::AndroidIntent;
use leptoskit_plugin_android_intent::{action, extra, flag, Extras, Intent};

let intents = AndroidIntent::new()?; // Unavailable outside Android.

// Upstream's README example: open a Play Store page with an extra.
let store = Intent::new()
  .action(action::VIEW)
  .data("https://play.google.com/store/apps/details?id=com.example")
  .extra("authAccount", "someone@example.com");
intents.launch(&store).await?;

// This app's settings page.
let settings = Intent::new()
  .action(action::APPLICATION_DETAILS_SETTINGS)
  .data("package:com.example.app");
intents.launch(&settings).await?;

// A broadcast only this app receives, with typed and array extras.
let refresh = Intent::new()
  .action("com.example.app.REFRESH")
  .package("com.example.app")
  .flag(flag::RECEIVER_FOREGROUND)
  .extra("count", 3)
  .extra("ids", vec![1_i64, 2, 3])
  .extra("filters", Extras::new().put("unread", true));
intents.send_broadcast(&refresh).await?;

// Share through a titled chooser.
let share = Intent::new()
  .action(action::SEND)
  .mime_type("text/plain")
  .extra(extra::TEXT, "Hello");
intents.launch_chooser(&share, "Share with").await?;

// Ask before offering a feature, subject to <queries>.
if let Some(found) = intents.resolved_activity(&store).await? {
  println!("{} ({})", found.label, found.package_name);
}

intents
  .parse_and_launch("intent://scan/#Intent;scheme=zxing;package=com.google.zxing.client.android;end")
  .await?;
```

## Security

Guest code can be replaced over the air, so a compromised update or a guest that forwards text from a web page must not be able to do more than the app already chose to allow. The Kotlin plugin is the trust boundary: it decodes every request itself and applies the same rules as the Rust builder, in the same order.

- **No URI grants.** `FLAG_GRANT_READ_URI_PERMISSION`, `FLAG_GRANT_WRITE_URI_PERMISSION`, `FLAG_GRANT_PERSISTABLE_URI_PERMISSION` and `FLAG_GRANT_PREFIX_URI_PERMISSION` are refused with `PermissionDenied`, on built intents and on parsed `intent:` URIs. With them, an intent could give any app access to this app's own content URIs, such as private files behind its `FileProvider`. Share files through a dedicated plugin that grants one URI to one user-chosen target.
- **No `file:` URIs.** Android throws `FileUriExposedException` for them anyway, and they would name private paths.
- **No capabilities in extras.** Extras are primitives, strings, arrays and bundles. There is no way to send a `Uri`, an `Intent`, a `PendingIntent` or any other parcelable, so a request cannot carry a grant or a redirect.
- **Browser rules for `intent:` URIs.** Parsed URIs cannot name a component or a selector and only reach activities that declare `CATEGORY_BROWSABLE`, so a URI that came from a web page cannot open this app's private activities.
- **Bounds.** Payloads are at most 128 KiB; names 512 bytes; URIs 8 KiB; string and byte extras 64 KiB; arrays 1,024 elements; 64 extras per bundle, 256 in total, nested at most 4 bundles deep; 8 categories; 1 KiB chooser titles and labels. Strings must be valid UTF-8, doubles finite, booleans exactly 0 or 1, and keys unique.

What remains is the app's own authority, and the plugin cannot narrow it: a guest can start any exported activity or service of any app, broadcast to any receiver, and reach this app's own components, exported or not, using every permission the app holds. A broadcast without a package can be read by any app that registers for its action. If your app holds permissions such as `CALL_PHONE`, an updated guest can use them through intents. Grant this plugin's methods only to the guests that need them, prefer `package` on broadcasts, and keep sensitive permissions out of apps that select it.

## Wire format and tests

Integers are little-endian u32 unless noted. A string is a u32 byte length and UTF-8 bytes, and an absent optional string is empty. An intent is version `1`, flags, action, data, MIME type, package, component class, a category count and categories, then a bundle: an entry count, then each entry's key, u32 tag (1 `bool` as a u32 word, 2 `i32`, 3 `i64`, 4 `f64` bits, 5 string, 6 bundle, 7 bytes, 8–12 boolean, int, long, double and string arrays, 13–14 integer and string lists) and value. A chooser request is a title string followed by an intent. `parse_and_launch` sends the URI's bytes. Replies are empty, a u32 boolean, or for `resolved_activity` `0`, or `1` followed by package, activity class and label strings.

The Rust tests cover every encoder and decoder, every bound and its off-by-one, every truncation, unknown tags, duplicate keys and malformed values, and check two golden byte arrays. `tests/android/AndroidIntentWireTest.kt` decodes those same golden bytes and repeats the rejection cases on a plain JVM (`kotlinc` with the bridge's `NativePlugin.kt`). `tests/android/AndroidIntentPluginTest.kt` runs in the generated Android plugin suite on an emulator: it resolves this app's own launcher activity and Settings, launches Settings and returns home, sends a broadcast with every extra type to a receiver the test registers, and checks the refusals for grants, `file:` URIs, components and selectors in `intent:` URIs, missing activities and missing services.

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_android_intent)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
