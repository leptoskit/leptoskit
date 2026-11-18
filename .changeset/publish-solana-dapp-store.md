---
"leptoskit_cli": minor
"leptoskit_config": minor
---

Publish to the Solana dApp Store. `[publish.solana]` adds Solana Mobile's store for Seeker and Saga phones to `leptoskit publish`: without `--submit` the run builds the release-signed APK with your dApp Store keystore, checks the listing text and images, the signing key (which must differ from Google Play's), the `arm64-v8a` build target, and the publishing wallet's balance (0.016 SOL minimum, through one RPC call that may only warn on an outage), and sends nothing, because Solana Mobile's official `dapp-store` tool has no draft stage; with `--submit` it runs that tool against the Publisher Portal with the APK, the wallet keypair, and "What's New" from `store/en-US/release_notes.md`, the portal API key held only in that one tool's environment. The keystore reuses the release-signing variables under `LEPTOSKIT_SOLANA_*` names, `leptoskit publish credentials` lists the six new credentials, and the generated release workflow sets up Node and decodes the keystore for it.
