---
"release": minor
---

# Linux apps get sign-in and the system plugins

The generated Linux app now registers `secure-store`, `web-auth`, `device`, `localization`, `network`, `battery`, and `linking`, besides the plugins written in Rust that it already ran.

- **Sign-in works on Linux.** Providers sign in through the default browser, which comes back to the app on a loopback address. The session is kept in the desktop's Secret Service keyring, such as GNOME Keyring or KWallet. Linux has no platform passkeys, so `passkeys` is left out there without a warning, and passkey calls answer `unsupported`.
- **New projects' Linux hosts get the plugins too.** `leptoskit new --auth`, `--db`, and `leptoskit generate auth` add sign-in's plugins, or the HTTP plugin, to `linux/Cargo.toml`, so a new app's Linux build reaches its server.
- **Snapshot plugins answer.** `device`, `localization`, `network`, and `battery` answer snapshots on Linux. Waiting for a battery or network change is still unavailable there. `linking` opens links in the default handler.
- **No zone is not an error.** `localization` reports no time zone, instead of failing, on a system with none configured, such as a container.
