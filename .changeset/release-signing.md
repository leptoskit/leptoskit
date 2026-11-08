---
"release": minor
---

# Signed release builds for the stores

`leptoskit build --release --platform android`, `quest`, and `ios` now make what each store takes, signed with material read only from the environment, so a secret manager can hand it to one build: `monosecret run --profile release -- leptoskit build --release --platform android`.

- **Android and Meta Quest** sign the App Bundle and the APK with your upload key, from `LEPTOSKIT_ANDROID_KEYSTORE` (or the keystore itself, base64-encoded, in `LEPTOSKIT_ANDROID_KEYSTORE_BASE64`, which is written to a private temporary file and removed afterwards), `LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD`, `LEPTOSKIT_ANDROID_KEY_ALIAS`, and `LEPTOSKIT_ANDROID_KEY_PASSWORD`. Gradle reads the passwords from its environment, never its command line. The build checks that both are signed and copies them to `.leptoskit/release/<target>/<name>-<version>.{aab,apk}`.
- **iOS** archives the app and exports `.leptoskit/release/ios/<name>-<version>.ipa` for the team in `LEPTOSKIT_IOS_TEAM_ID` (or `native.apple_team_id`). Automatic signing signs in with the App Store Connect key `leptoskit publish` uses, or the account in Xcode; manual signing names `LEPTOSKIT_IOS_CODE_SIGN_IDENTITY` and `LEPTOSKIT_IOS_PROVISIONING_PROFILE`. `[native.ios]` sets `signing-style` and `export-method` (`app-store-connect`, `ad-hoc`, or `development`), and `--export-method` overrides it for one build.
- `--build-number <n>` sets Android's `versionCode` and iOS's `CFBundleVersion`.

A release never falls back to an unsigned build or the debug key. Without its signing material it stops before compiling and names every missing variable; a keystore or key file inside the project's repository, Android's `debug.keystore`, and the `androiddebugkey` alias are refused. `--unsigned` builds an unsigned release on purpose, as earlier versions did, for measuring and testing. Debug builds and simulator builds are unchanged.
