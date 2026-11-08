# Signing a release

A store takes only a build signed with your key, and the key is the one thing about an app that must never leak: anyone who holds it can ship updates as you. So the keystore and its passwords shouldn't sit in the project, and a release build shouldn't quietly come out unsigned, or signed with the debug key, because a variable was missing.

`leptoskit build --release` for Android, Meta Quest, and iOS makes what each store takes, signed with material it reads only from the environment. Without it, the build stops before anything compiles and names what is missing.

## An example

Notes ships to Google Play and to the App Store. Its upload keystore and passwords live in a secret manager, and are handed to one build at a time. With [Monosecret](secrets.md), declare them in a profile of `monosecret.toml`, and keep the values in your keyring or your team's provider:

```toml
[profiles.release]
LEPTOSKIT_ANDROID_KEYSTORE_BASE64 = { description = "The upload keystore, base64-encoded", required = true }
LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD = { description = "The upload keystore's password", required = true }
LEPTOSKIT_ANDROID_KEY_ALIAS = { description = "The upload key's alias", required = true }
LEPTOSKIT_ANDROID_KEY_PASSWORD = { description = "The upload key's password", required = true }
```

Then build through it:

```bash
monosecret run --profile release -- leptoskit build --release --platform android
```

```text
✔ signed .leptoskit/release/android/notes-1.4.0.aab
✔ signed .leptoskit/release/android/notes-1.4.0.apk
```

Without the profile, the same build stops straight away, before compiling anything:

```text
error: a release Android build is signed with your upload key, and these aren't set:
LEPTOSKIT_ANDROID_KEYSTORE (or LEPTOSKIT_ANDROID_KEYSTORE_BASE64), LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD,
LEPTOSKIT_ANDROID_KEY_ALIAS, LEPTOSKIT_ANDROID_KEY_PASSWORD. Set them in the environment, …
```

Any tool that sets environment variables for one command works the same way.

## What comes out

| Platform   | Command                                        | Artifacts, in `.leptoskit/release/`                                                | For                                                                                                     |
| ---------- | ---------------------------------------------- | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Android    | `leptoskit build --release --platform android` | `android/<name>-<version>.aab` and `android/<name>-<version>.apk`                  | The App Bundle for Google Play; the APK for sideloading and other stores, such as the Solana dApp Store |
| Meta Quest | `leptoskit build --release --platform quest`   | `quest/<name>-<version>.apk` and `quest/<name>-<version>.aab`                      | The APK for the Meta Horizon Store                                                                      |
| iOS        | `leptoskit build --release --platform ios`     | `ios/<name>-<version>.ipa`, with `ios/App.xcarchive` and `ios/ExportOptions.plist` | TestFlight and the App Store, or the devices of an ad hoc or development profile                        |

`<name>` is `app.name` and `<version>` is `app.version`. Android's own copies stay where Gradle writes them, in `android/app/build/outputs/bundle/release/` and `android/app/build/outputs/apk/release/`. Each release clears the previous one's outputs first, so what is there is what this build made.

Every store upload needs a build number higher than the last one. `--build-number 42` sets it: Android's `versionCode` and iOS's `CFBundleVersion`. The version people see is `app.version`. [`leptoskit publish`](24-publishing.md) picks the next build number itself.

## Android and Meta Quest

The Android and Quest builds sign with your upload key. Make one once, and keep it outside the project:

```bash
keytool -genkeypair -keystore upload.jks -alias upload -keyalg RSA -keysize 2048 -validity 10000
```

| Variable                              | What it is                                                      |
| ------------------------------------- | --------------------------------------------------------------- |
| `LEPTOSKIT_ANDROID_KEYSTORE`          | The keystore's path, outside the project's repository           |
| `LEPTOSKIT_ANDROID_KEYSTORE_BASE64`   | Or the keystore itself, base64-encoded (`base64 -i upload.jks`) |
| `LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD` | The keystore's password                                         |
| `LEPTOSKIT_ANDROID_KEY_ALIAS`         | The key's alias, `upload` above                                 |
| `LEPTOSKIT_ANDROID_KEY_PASSWORD`      | The key's password                                              |

A secret manager holds text, so give it the base64 form. The build writes it to a new file in the system's temporary directory, readable only by you, and removes it when the build ends.

What the build does:

1. Before compiling, it checks every variable is set, and refuses a keystore inside the project's repository (even one git ignores), Android's `debug.keystore`, and the debug key's alias, `androiddebugkey`.
2. It checks `android/app/build.gradle.kts` reads `LEPTOSKIT_ANDROID_KEYSTORE` and doesn't sign with the debug signing config.
3. It runs Gradle's `assembleRelease` and `bundleRelease`. Gradle reads the passwords from the environment itself, so they never appear on a command line, where other programs on the machine could read them, or in the build's output.
4. It checks the APK carries an APK signature and the App Bundle is signed, and fails if Gradle made an unsigned APK instead.

The build is signed with your upload key. Google Play re-signs what it installs with the app signing key it keeps (Play App Signing). Other stores, and people who sideload the APK, see your key, so keep using the same one for every update.

Projects made by `leptoskit new` already read the key in their Gradle script. A project made before that needs the `signingConfigs` block in `android/app/build.gradle.kts`, inside `android { … }`, and the release build type pointed at it:

```kotlin
signingConfigs {
    val keystore = providers.environmentVariable("LEPTOSKIT_ANDROID_KEYSTORE").orNull

    if (keystore != null) {
        create("release") {
            storeFile = file(keystore)
            storePassword = providers.environmentVariable("LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD").orNull
            keyAlias = providers.environmentVariable("LEPTOSKIT_ANDROID_KEY_ALIAS").orNull
            keyPassword = providers.environmentVariable("LEPTOSKIT_ANDROID_KEY_PASSWORD").orNull
        }
    }
}
buildTypes {
    release {
        signingConfig = signingConfigs.findByName("release")
    }
}
```

The Quest build signs with the same variables and the same Gradle script, and writes its own folder, `.leptoskit/release/quest/`, so an Android release and a Quest release don't replace each other.

## iOS

The iOS release archives the app with `xcodebuild archive`, then exports an `.ipa` with `xcodebuild -exportArchive`. It runs on a Mac with Xcode.

The settings that aren't secret go in `leptoskit.toml`:

```toml
[native]
apple_team_id = "ABCDE12345"

[native.ios]
signing-style = "automatic" # or "manual"
export-method = "app-store-connect" # or "ad-hoc", or "development"
```

| Setting                    | Default               | What it does                                                                                                                                                                        |
| -------------------------- | --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `native.ios.signing-style` | `"automatic"`         | `automatic` lets Xcode pick the certificate and make the provisioning profile; `manual` names both.                                                                                 |
| `native.ios.export-method` | `"app-store-connect"` | `app-store-connect` for TestFlight and the App Store, `ad-hoc` for the devices in the profile, `development` for development devices. `--export-method` overrides it for one build. |

The rest comes from the environment:

| Variable                             | When                              | What it is                                                                |
| ------------------------------------ | --------------------------------- | ------------------------------------------------------------------------- |
| `LEPTOSKIT_IOS_TEAM_ID`              | Always, or `native.apple_team_id` | The team's 10-character ID, from developer.apple.com under **Membership** |
| `LEPTOSKIT_IOS_SIGNING_STYLE`        | Optional                          | `automatic` or `manual`, over `native.ios.signing-style`                  |
| `LEPTOSKIT_PUBLISH_APPLE_ISSUER_ID`  | Automatic signing, with a key     | The App Store Connect API key's issuer ID                                 |
| `LEPTOSKIT_PUBLISH_APPLE_KEY_ID`     | Automatic signing, with a key     | The key's ID                                                              |
| `LEPTOSKIT_PUBLISH_APPLE_KEY`        | Automatic signing, with a key     | The `.p8` file's contents, or its path outside the project's repository   |
| `LEPTOSKIT_IOS_CODE_SIGN_IDENTITY`   | Manual signing                    | The certificate, such as `Apple Distribution`, or its SHA-1               |
| `LEPTOSKIT_IOS_PROVISIONING_PROFILE` | Manual signing                    | The provisioning profile's name or UUID                                   |

**Automatic signing** is the simplest. With the three App Store Connect key variables, the same ones [`leptoskit publish app_store`](24-publishing.md#the-app-store) uses, Xcode signs in with the key and makes the certificate and profile it needs, so it works on a build machine with nothing installed. Make the key in App Store Connect under **Users and Access**, **Integrations**, **App Store Connect API**, with the App Manager role. Without the key, Xcode uses the account signed in under **Xcode**, **Settings**, **Accounts**. A key given as text is written to a private temporary file for Xcode, and removed when the build ends.

**Manual signing** names a certificate and a provisioning profile you've already installed in this Mac's keychain and in `~/Library/MobileDevice/Provisioning Profiles`. The profile must be for `native.package_id` and match the export method: an App Store profile for `app-store-connect`, an ad hoc one for `ad-hoc`.

A device build without `--release` is still unsigned, for running from Xcode, and simulator builds are signed ad hoc, as before.

## An unsigned release, on purpose

Measuring an app's size, or testing a release build on an emulator, doesn't need your key. `--unsigned` asks for an unsigned build, exactly as earlier versions made it:

```bash
leptoskit build --release --platform android --unsigned
```

No store takes it. `--unsigned` is never chosen for you: a release without its signing material stops instead.

## What doesn't work yet

- **Manual signing with a Safari extension, or with plugins that use CocoaPods:** manual signing names one provisioning profile, which Xcode applies to every target it builds, and the extension needs its own while the pods' targets refuse one. The build says so before compiling; use automatic signing for such an app.
- **Uploading to the Meta Horizon Store and the Solana dApp Store:** upload the APK with each store's own tools. `leptoskit publish` uploads to Google Play and the App Store.
- **A killed build** leaves a base64 keystore or an App Store Connect key it wrote in the system's temporary directory, readable only by you. A build that ends, successfully or not, removes it.
