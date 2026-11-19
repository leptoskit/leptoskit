---
"leptoskit_cloud": minor
"leptoskit_cli": patch
---

# Publish to app and extension stores from the cloud

`leptoskit_cloud::publishing` adds `publish`, which builds an uploaded archive for its stores and uploads it on one of the organisation's build workers, as `lk publish` does, with the environment's secrets named as `lk publish` reads them. `publication`, `publications` and `publication_log` read a publish back, with its report and its log. A publish is never run twice, since it may have reached a store. `JobKind`, `JobInput` and `JobOutcome` gain `Publish`; store credentials may now be set as secrets, and no release is given them.

`leptoskit publish` takes a keystore's `_BASE64` variable in place of its path for Google Play and the Solana dApp Store, and with `LEPTOSKIT_PUBLISH_CONTENTS_ONLY=1` refuses a credential that names a file.
