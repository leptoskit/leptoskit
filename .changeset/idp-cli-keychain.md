---
"release": minor
---

# Keep the generated CLI's sign-in in the keychain

A CLI from `leptoskit rpc generate --target cli` whose functions need scopes now keeps its tokens in the operating system's keychain: the macOS Keychain, Windows Credential Manager, or the Secret Service on Linux, through the `keyring` crate. Where no keychain answers, such as a server without a desktop session, `login` says so and keeps them in the `0600` file in the config directory as before. Saving to the keychain removes a file an older version left, renewal still takes turns behind the lock file, and `logout` forgets the tokens from both places, even when the stored sign-in is damaged.
