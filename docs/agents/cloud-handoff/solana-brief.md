# Publish: Solana dApp Store (specification)

Adds one store, the **Solana dApp Store**, to `leptoskit publish`.

## Read first

- `research-store-apis.md` (same directory as this file): section 1 ("Solana dApp Store") in full, and the "Solana dApp Store" paragraph under "Upload and submit sequences". It is the source for every store fact; do not research further and do not contact any store.
- `codebase-map.md` (same directory): section 7 ("Publish store client shape"): every place a new store touches.
- In the repository: `AGENTS.md`; `docs/agents/publishing-plan.md`; `crates/leptoskit_cli/src/publish/mod.rs`, `play.rs`, `macos.rs`, `tools.rs`, `secrets.rs`, `steps.rs`, `images.rs`, `listing.rs`, `tests.rs`; `crates/leptoskit_cli/src/native/signing.rs`; `crates/leptoskit_config/src/publish.rs`.

## What the store requires

- Publishing an update is done by Solana Mobile's official command-line tool `@solana-mobile/dapp-store-cli` (npm): `dapp-store --apk-file <path> --keypair <solana-keypair.json> --whats-new "<text>"`, with the portal API key in the environment variable `DAPP_STORE_API_KEY`. One invocation ingests the APK, mints the release NFT, and submits the release for review. The app is matched by the Android package name inside the APK and must already exist in the Publisher Portal.
- The artifact is a release-signed **APK** (not an App Bundle), containing `arm64-v8a`, at most 2 GB, signed with a key that is **not** the Google Play key.
- Each update needs a changed `versionName`, a `versionCode` one higher than the store's, and "What's New" text.
- The signing wallet needs at least 0.016 SOL (16,000,000 lamports).
- Listing limits: name at most 25 characters, short description at most 50, icon 512x512 without transparency, banner 1200x600, four to eight previews of identical dimensions, images at most 3 MB.

## Design (decided)

- **Configuration:** `[publish.solana]` in `leptoskit_config::publish`: no required keys. Optional `rpc_url` (HTTPS; default `https://api.mainnet-beta.solana.com`) used only for the balance check, and `cli_version` (default a constant pinned to `1.0.1`; validated as a plain semantic version, never a range or tag). Store key `"solana"`, placed after `"play"` in `STORES`. It requires the `android` platform; the validation message says so.
- **Credentials (`NEEDED`):** `LEPTOSKIT_PUBLISH_SOLANA_API_KEY` (the portal API key), `LEPTOSKIT_PUBLISH_SOLANA_KEYPAIR` (a path to a Solana CLI keypair JSON file, or the JSON array itself), and a dApp Store signing keystore separate from Play's: `LEPTOSKIT_SOLANA_KEYSTORE` (or `LEPTOSKIT_SOLANA_KEYSTORE_BASE64`), `LEPTOSKIT_SOLANA_KEYSTORE_PASSWORD`, `LEPTOSKIT_SOLANA_KEY_ALIAS`, `LEPTOSKIT_SOLANA_KEY_PASSWORD`. Each with a `how` sentence saying where to create it. The keystore-reading code in `native/signing.rs` is reused by parameterising it over the variable prefix.
- **Upload versus submit.** The official tool has no draft stage: it submits for review. So `leptoskit publish solana` without `--submit` builds the signed APK, runs every check, and stops, saying that the next step is `--submit` and that nothing was sent. With `--submit` it runs the tool.
- **Build:** the release APK through the existing Android release build path and the `Runner` seam, with the Solana keystore's values supplied to Gradle in place of the Play ones for that invocation only (under the `LEPTOSKIT_ANDROID_*` names in that one tool's environment, never the process environment). The APK is copied to `.leptoskit/publish/solana/`.
- **Running the tool:** through the `Runner` seam as `npx --yes @solana-mobile/dapp-store-cli@<cli_version> --apk-file … --keypair … --whats-new …`, with `DAPP_STORE_API_KEY` passed only in that tool's environment as a `Secret`. A keypair given as JSON is written to a private temporary file (mode 0600, private directory, removed on drop) and that path is passed. A keypair file inside the project that Git does not ignore is refused. The tool's output must not leak the key.
- **Checks (in `--dry-run` and before any submit):** the listing limits from `store/` (name, summary as the short description, release notes present); images from `store/screenshots/` folders added to `images.rs`'s `TARGETS` (`solana-icon` 512x512 opaque, `solana-banner` 1200x600, `solana-previews` four to eight images with identical dimensions, each at most 3 MB); the Solana keystore is not the Play keystore (refused when both resolve to the same file or the same base64 content); the build includes `arm64-v8a`; the keypair parses as a 64-byte array; and, only when credentials are present and not `--dry-run`, the wallet balance through one JSON-RPC `getBalance` call over the existing `Http` seam against `rpc_url` (base58 address from the keypair's public half, using an existing workspace base58 dependency if there is one). A balance under 16,000,000 lamports fails with the address to fund and the amount. An RPC outage is a warning, not a failure.
- **Steps ("What to do next"):** the person and agent steps the research lists (portal account, publisher profile, KYC/KYB, connecting the publisher wallet, the app details form and the App NFT mint approval, creating the API key, funding the wallet, the decision to release, what to do after a rejection), each with its URL, what to enter from `store/`, why the tool cannot do it, and `done_when`. After a successful submit, the summary says the release is in review and gives the documented review times.
- `leptoskit publish credentials`, the generated release workflow (`write_workflow`, template `crates/leptoskit_cli/src/new_project/templates/release.yml`, with the store's secrets and a Node setup step gated on the store, action pinned to a commit), and `icon_step` handle the new store wherever they iterate stores. The unit test that keeps the workflow passing every credential in `NEEDED` stays true.

## Documentation

- `docs/src/guide/24-publishing.md`: a "Solana dApp Store" section in the same voice and shape as the other stores.
- `skills/leptoskit/references/publishing.md`: the store's dashboard pages in order and its common rejections.
- `docs/agents/publishing-plan.md`: a "Progress" entry stating exactly what is built and what was not run for real (no real store was contacted; the official tool was not executed), and the store moved out of any "Later" list.
- `docs/src/readme.md` "Where things stand" or a platform page, if it lists stores.

## Tests

In the style of `publish/tests.rs`: `Recorder` for the `Runner` seam asserting the exact `npx` argument list, that the API key appears only in that tool's environment, and that no secret (API key, keypair bytes, keystore password) appears in any output, error, `Debug` string, or the written report; a fake JSON-RPC server through the `store(...)` helper for the balance check (enough, too little, outage); every check's refusal; the no-`--submit` path sending nothing; config validation; a dry-run snapshot. The schema is regenerated (`devenv shell schema:update`) and every changed `.snap` reviewed.

## Acceptance

- `cargo nextest run -p leptoskit_cli publish` and `-p leptoskit_config --all-features` pass.
- `cargo clippy -p leptoskit_cli --all-features --all-targets -- -D warnings` and the same for `leptoskit_config` pass.
- `devenv shell schema:check`, `lint:monostyle`, `lint:format`, `lint:releases`, `lint:docs`, and `deny:check` pass.
- A changeset for `leptoskit_cli` and `leptoskit_config` (minor), mirroring an earlier publishing changeset's frontmatter.

## Leave alone

Every other store's behaviour and tests, the cloud crates, and anything not named above. Do not re-implement the portal's HTTP procedures in Rust: wrapping the official tool is the decision.
