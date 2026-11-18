# Store publishing APIs: research for `leptoskit publish` and a hosted cloud publisher

Research date: **2026-11-18**. Every URL below was read on that date unless marked otherwise.
Read-only research: no repository file was changed, no account was created, no authenticated API was called, no tool was installed.

## How to read this file

- **Existing design** (from `docs/agents/publishing-plan.md` and `crates/leptoskit_cli/src/publish/`): two steps per store, **upload** (draft / test track / internal) and **submit** (review or release); credentials from environment only, held in a redacting `Secret`; every run ends with "What to do next" steps marked `agent` or `person`; clients are small `ureq` wrappers (`http.rs`, `google.rs`, `play.rs`, `app_store.rs`, `chrome.rs`, `edge.rs`, `firefox.rs`, `macos.rs`) tested against a fake server. Already covered: Chrome Web Store, Edge Add-ons, AMO, App Store Connect (iOS/TestFlight/review), macOS Developer ID, Google Play. A `leptoskit_platform_oculus` and `leptoskit_platform_harmonyos` crate exist.
- **UNVERIFIED** = I could not confirm it from an official source on 2026-11-18. Do not implement against it without checking.
- **(secondary)** = from a blog, forum or search summary, not from the store's own documentation.
- Source quality caveats: Huawei's developer site returned HTTP 503 for several pages (upload-url, Publishing API guide), and its HarmonyOS guide pages are 600 KB table-of-contents shells that the fetcher could not read past. Page text was summarised by a small model, so quoted endpoint paths were cross-checked against a second source where I could. Where I could not, I say so.
- **Prompt-injection check:** no fetched page asked me to identify myself, add headers, or contact anyone. Nothing to report.
- **Two repo-relevant findings outside the stores list:**
  1. Apple broke password-based (Apple ID + app-specific password) uploads around **2026-10-28** (error `-22938`, cause: a stricter `User-Agent` check on the legacy upload endpoint). Source (secondary): https://dev.to/xxhshenqi/apple-blocked-app-specific-password-uploads-error-22938-heres-how-to-upload-ipas-with-the-5fej . leptoskit's existing App Store path uses an API key plus `xcodebuild -exportArchive`, which that article says is unaffected (Xcode and Transporter users are unaffected). A hosted service on Linux cannot use `xcodebuild`, so it needs the Build Uploads API (see Mac App Store section).
  2. The Solana dApp Store's old config/NFT CLI flow **stopped working in April 2026** (details in section 1).

---

# Part A: store by store

## 1. Solana dApp Store (Solana Mobile; Seeker and Saga)

### (a) Official docs and repos

| What | URL | Read |
|---|---|---|
| Submit a new app (portal) | https://docs.solanamobile.com/dapp-store/submit-new-app | 2026-11-18 |
| Submit an update (portal) | https://docs.solanamobile.com/dapp-store/submit-an-update | 2026-11-18 |
| dApp Publishing CLI (portal-backed) | https://docs.solanamobile.com/dapp-store/publishing-cli (also `/dapp-publishing/submit`) | 2026-11-18 |
| Listing page guidelines | https://docs.solanamobile.com/dapp-store/listing-page-guidelines.md | 2026-11-18 |
| Build and sign an APK | https://docs.solanamobile.com/dapp-store/build-and-sign-an-apk.md | 2026-11-18 |
| Publishing from Google Play | https://docs.solanamobile.com/dapp-store/publishing-from-google-play.md | 2026-11-18 |
| Publisher Policy | https://legal.solanamobile.com/publisher-policy-web (308 from solanamobile.com/publisher-policy-web) | 2026-11-18 |
| Docs index | https://docs.solanamobile.com/llms.txt | 2026-11-18 |
| Portal | https://publish.solanamobile.com ; API keys at https://publish.solanamobile.com/dashboard/settings/api-keys | 2026-11-18 |
| CLI + core library repo | https://github.com/solana-mobile/dapp-publishing (monorepo: `packages/cli`, `packages/core`) | 2026-11-18 |
| Old repo URL `solana-mobile/dapp-store-publishing-tools` | returns **404** (moved/renamed) | 2026-11-18 |

### Is the old CLI/NFT flow still current? **No.**

- The older flow was `dapp-store init` (writes `config.yaml`), then `create publisher`, `create app`, `create release`, `validate`, and `publish submit | update | remove | support`, minting Publisher, App and Release NFTs on Solana mainnet, with assets on Arweave (Turbo) or S3 (secondary: https://learn.blueshift.gg/en/courses/dapp-store-publishing/solana-dapp-store, https://www.helius.dev/blog/publishing-solana-mobile-apps; those guides are now outdated).
- **Publisher NFT removed:** release notes of `v0.12.0`, published **2025-09-05**, list "Remove publisher nft" (https://api.github.com/repos/solana-mobile/dapp-publishing/releases).
- **Legacy API dead since April 2026:** PR #331 "feat(core)!: remove the legacy pre-portal publishing API" (opened 2026-10-17, **merged 2026-10-26**, commit `853ddd5`) says the legacy API "stopped working against the dApp Store in April 2026". It removes `createApp`, `createRelease`, `validateApp`, `validateRelease`, the JSON schemas, the `publishSubmit/Update/Remove/Support` no-ops, `CoreUtils.mintNft`, and the `example/` config. Replacements named: `createPortalClient`, `loadKeypairSignerFromFile`, `publishRelease`. https://github.com/solana-mobile/dapp-publishing/pull/331
- **README today:** the `init`, `create`, `validate` and direct `publish submit|update|remove|support` commands are "no longer part of the active CLI surface" (https://github.com/solana-mobile/dapp-publishing).
- **Docs inconsistency:** https://docs.solanamobile.com/dapp-publishing/publishing-updates still describes the old `config.yaml` + release-NFT + `npx dapp-store publish update` flow with no deprecation notice. Treat that page as stale.
- I found **no dated announcement** of the portal launch. Portal launch date: UNVERIFIED (between the April 2026 legacy shutdown and the 1.0.0 CLI).

### Current publishing model (two surfaces, one backend)

1. **Publisher Portal** (web, manual): sign up, create publisher profile (individual or organisation), **KYC (individual) or KYB (company)**, connect the publisher wallet, fill in app details (Save **mints the App NFT**, wallet approval), then Releases > New Version, upload a signed APK or give an APK URL, "What's New", Submit. "The portal processes the APK and publishes the release onchain. Approve every wallet signing request." Storage defaults to **Cloudflare R2**, switchable to **ArDrive** or **AWS S3** under Settings > Preferences. https://docs.solanamobile.com/dapp-store/submit-new-app
2. **`@solana-mobile/dapp-store-cli`** (portal-backed, for updates of an app that already exists in the portal): `npm install -g @solana-mobile/dapp-store-cli`, then `dapp-store --apk-file <path> | --apk-url <url> --keypair <solana-cli-keypair.json> --whats-new "<text>"`. API key from env `DAPP_STORE_API_KEY` or stdin. The app is matched by the Android package name read from the APK. Prerequisites: app already created in the portal with its App NFT minted, release-signed APK, Solana CLI keypair file, portal API key. https://docs.solanamobile.com/dapp-store/publishing-cli
   - npm `dist-tags.latest` = **1.0.1** (https://registry.npmjs.org/@solana-mobile/dapp-store-cli). Bump PR #329 merged **2026-09-14** (commit `3b7f4be`), 9 test suites / 64 tests. License Apache-2.0 (jsDelivr, secondary).
   - GitHub "latest release" list shows `v0.13.0` (2025-09-07) as newest tag; 1.0.x is not tagged as a GitHub Release (observed 2026-11-18). Repo `pushed_at` 2026-10-26.

### (b) Account requirements and fees

- Portal account with email, **KYC/KYB must pass before you can add a dApp** (can be skipped at profile creation). Stuck? Developer role in Discord `#developer`, then a "KYC/KYB Verification Issue" ticket in `#dev-answers`.
- Publisher wallet: "Use a new wallet dedicated to publishing"; it is "required for all future submissions of this app. Losing access blocks future submissions."
- **SOL cost:** the CLI refuses to start unless the signer holds ≥ **16,000,000 lamports (0.016 SOL)**; source comment: release NFT minting "currently needs roughly ~0.015 SOL in rent plus fees" (`packages/core/src/publication/fundingPreflight.ts`, https://raw.githubusercontent.com/solana-mobile/dapp-publishing/main/packages/core/src/publication/fundingPreflight.ts). Default RPC `https://api.mainnet-beta.solana.com` (429 retried after 500/1000/2000 ms; an RPC outage downgrades to a warning). The docs state no listing or storage fee; the portal's own storage charge, if any: UNVERIFIED. No revenue-share figure on the docs read; the Google Play comparison page says "no 30% tax on in-app purchases".
- No annual fee documented. Publisher Policy and Developer Agreement must be accepted.

### (c) Artifact and signing

- **APK only** (not AAB). Must be a **signed release build**; "Debug builds are rejected"; "must support `arm64-v8a` and be at most 2 GB" (submit-new-app page). The build-and-sign page does not mention arm64-v8a, so only the submit page states it.
- **Signing key must differ from Google Play's:** "You cannot use the same signing key for both Google Play and the dApp Store" (publishing-from-google-play page). Make a dedicated keystore (RSA 2048, alias e.g. `dappstore`, 10 000 days). Updates must use the same key: "losing them means you cannot update your app". Verify with `apksigner verify --print-certs`. Play App Signing is not mentioned on those pages.
- Updates: same package name, same signing key, `versionName` changed, **`versionCode` incremented by one monotonically**, "What's New" required (shown to users).
- **Listing assets** (listing-page-guidelines): icon **512x512**, background colour, no transparency; banner **1200x600**; **4 to 8** screenshots or videos; images JPG/PNG/GIF/WebP ≤ 3 MB each; videos `.mp4` ≤ 30 MB, 1080p recommended; first preview portrait 9:16 or landscape 16:9 and every other preview the same dimensions; ≥ 1080 px recommended. Name ≤ **25** chars; short description ≤ **50** chars; long description no stated limit. Plus: public website, support and contact email, Terms of Use URL and Privacy Policy URL (each a public HTTPS page the portal checks).
- **Package name** must match the APK's application ID exactly, is case-sensitive, unique, and "can't be changed later".

### (d) Authentication for automation

- **Portal HTTP auth:** header `x-api-key: <portal API key>` (key created by hand in the dashboard). Base URL derived as `<portal>/api`, so production is **`https://publish.solanamobile.com/api`**; requests go to **`${base}/trpc/<procedure>`**. `GET` for queries (`?input=<JSON>`), `POST` with a JSON body for mutations; responses may be batched (`{"0": ...}`) and may be a tagged `{_tag: 'Left'|'Right'}`. Sources: https://raw.githubusercontent.com/solana-mobile/dapp-publishing/main/packages/core/src/portal/client/http.ts and `.../publication/portalTargets.ts`.
- **Status of that API: UNVERIFIED as a public contract.** It is the portal's internal tRPC surface that the CLI uses; Solana Mobile documents only the CLI and the library (`createPortalClient`, `publishRelease`). Pin to the library, not to raw procedures, or accept breakage.
- No token lifetime applies to the API key (no expiry documented: UNVERIFIED).

### (e) The publish sequence the client runs (from `workflowClient.ts`)

Source: https://raw.githubusercontent.com/solana-mobile/dapp-publishing/main/packages/core/src/portal/client/workflowClient.ts . Procedure names are exact; response shapes live in `types.ts` (not read), so treat bodies as UNVERIFIED beyond the fields listed.

| Step | Procedure (kind) | Notes |
|---|---|---|
| 1 | `publication.createUploadTarget` (mutation) | body `fileHash`, `fileExtension`, `contentType`; returns an `uploadUrl` |
| 2 | HTTP `PUT <uploadUrl>` with `content-type` and the raw bytes | no API key sent to the storage host |
| 3 | `publication.finalizeUpload` (mutation) | same fields |
| 4 | `publication.createIngestionSession` (mutation, retried up to 3 times on 502/503/504-like errors, 1.5 s then 3 s) | body `source` = `portalUpload{releaseFileUrl,releaseFileName,releaseFileSize}` or `existingRelease{sourceReleaseId}` or `externalUrl{apkUrl,releaseFileName}`, plus `whatsNew`, `idempotencyKey`, optional `dappId` |
| 5 | `publication.getIngestionSession` (query, `sessionId`) | poll until the APK is processed |
| 6 | `publication.getPublicationBundle` (query, `releaseId`), `publication.getPublicationSession` (query, `publicationSessionId`, `releaseId`) | release metadata JSON is hashed (SHA-256) and uploaded through steps 1 to 3 when `nftMetadataUri` is missing |
| 7 | `publication.prepareReleaseNftTransaction` (mutation) | **server returns a prepared Solana transaction** |
| 8 | `publication.submitSignedTransaction` (mutation) `{signedTransaction, publicationSessionId}` | **the signed transaction goes back** |
| 9 | `publication.saveReleaseNftData` (mutation) | |
| 10 | `publication.prepareVerifyCollectionTransaction` then `publication.submitSignedTransaction` then `publication.markReleaseCollectionAsVerified` `{releaseId}` -> `{success, releaseId}` | collection verification |
| 11 | `publication.submitToStore` (mutation) `{releaseId, whatsNew, criticalUpdate, testingInstructions, isResubmission, attestation:{payload, requestUniqueId}}` | enters review |
| cleanup | `publication.cleanupRelease` | |

### The keypair's role, and whether signing can be delegated

- The release NFT is minted on **Solana mainnet** with the **publisher wallet** as signer and fee payer (it pays the ~0.015 SOL rent). The wallet is also the publisher's identity: "required for all future submissions of this app".
- **Attestation** (`packages/core/src/portal/attestation.ts`, https://raw.githubusercontent.com/solana-mobile/dapp-publishing/main/packages/core/src/portal/attestation.ts): the client fetches the latest `slot` and `blockhash`, builds `{"slot_number":…, "blockhash":"…", "request_unique_id":"<32 digits>"}`, signs its UTF-8 JSON bytes with the wallet's `signMessages`, requires a **64-byte signature**, and sends base64 of `signature(64 bytes) || JSON` as `attestation.payload`. This is a proof that the publisher wallet authorised this submission now; it is **not** the "publisher attests…" policy text. The Publisher Policy itself has **no general attestation clause** (https://legal.solanamobile.com/publisher-policy-web); it asks for compliance documents for regulated finance or medical apps, a compliant privacy policy, and UGC moderation.
- **Delegation is architecturally possible.** The core library signs through a `PublicationSigner` interface (message signer + transaction signer) and the server hands over *prepared transactions* and takes *signed transactions* back. The CLI only ships a keypair-file signer (`loadKeypairSignerFromFile`; no Ledger signer is shown in `cli/signer.ts`). So a hosted service can: (1) hold only the portal **API key**, (2) run steps 1 to 6 and 9 to 10's server calls, (3) at steps 7, 8, 10 and the attestation, **pause and ask the developer's wallet to sign** (a browser wallet through the wallet adapter, Seed Vault, or a Ledger) and resume. This needs a thin custom `PublicationSigner`; nothing in the docs prescribes it. Rated feasible, UNVERIFIED in production.
- **Alternative: server-held keypair.** Technically trivial (CLI takes a keypair file) but the docs advise a dedicated, minimally funded wallet. If a hosted service holds one it should be a wallet that holds only ~0.02 SOL and is the publisher wallet registered in the portal; loss or compromise lets an attacker publish releases for that app (they also need the portal API key and the APK signing key for a valid update).

### (f) Must be done once by hand

Sign-up with email code; publisher profile; **KYC/KYB**; connecting the publisher wallet; app details form including the first **App NFT mint** (wallet transaction); the first release of a *new* app is documented only through the portal UI (the CLI "requires an app already created in the portal"); creating the portal **API key**; listing assets and legal URLs; Discord role for support. Sources: submit-new-app and publishing-cli pages above.

### (g) Linux

CLI is Node.js (older docs: Node 18+, PNPM, Android SDK build tools; for 1.0.x UNVERIFIED), pure HTTPS plus Solana RPC. **Runs on a plain Linux server**, no browser needed after the one-time setup. Interactive wallet signing is only needed if signing is delegated.

### (h) Rate limits and pitfalls

Public mainnet RPC returns 429 (client retries); use a paid RPC for a hosted service (the CLI accepts an `rpcUrl`; flag name UNVERIFIED). Solana Mobile's portal rate limits: UNVERIFIED. Pitfalls: different signing key than Play; versionCode must rise by exactly +1 (docs say "incremented by one monotonically"); listing-only edits "create a new release with the same APK"; publishing listing changes and a new APK together requires keeping the listing as a draft; debug APK rejected; stale docs page still shows the dead NFT flow.

### (i) Review times

New app **3 to 4 business days**; updates **1 to 2 business days**; reviewed in submission order; result by email from `publishersupport@dappstore.solanamobile.com` (a Solana Mobile address, not the user's); no reply after 5 business days: App Review Inquiry in Discord `#dev-answers`. Common rejections: unreachable or off-topic publisher site, broken or non-public ToS/privacy links. Status check by API: `getIngestionSession` / `getPublicationSession` expose processing state; a "review status" procedure is **UNVERIFIED** (the portal shows "In Review").

---

## 2. Huawei AppGallery Connect

Docs: https://developer.huawei.com/consumer/en/doc/AppGallery-connect-References/agcapi-app-file-info-0000001111685202 ; https://developer.huawei.com/consumer/en/doc/development/AppGallery-connect-References/agcapi-app-submit-0000001158245061 ; https://developer.huawei.com/consumer/en/doc/app/agc-help-connect-api-obtain-server-auth-0000002271134661 ; upload API https://developer.huawei.com/consumer/en/doc/development/AppGallery-connect-References/agcapi-upload-file-new-0000001111845090 (503 on 2026-11-18; facts below from a secondary summary); community reference https://agccli.app/en/reference/publishing/ (secondary); HarmonyOS release https://developer.huawei.com/consumer/en/doc/harmonyos-guides/ide-publish-app (unreadable by the fetcher: 600 KB shell).

### (b) Account and fees

- Developer registration with identity verification (individual or company; company uses ID plus a bank document; secondary). Real-name verification is compulsory in mainland China. **Fee: UNVERIFIED** (I believe registration is free; not confirmed from an official page).
- **API client:** the account holder creates it in AGC > Users and permissions > Connect API > Create. "The API client can be managed only by your team account holder." **Team-level** client: leave Project at **N/A**; choosing a project makes calls fail: "If the value is not N/A, result code 403 will be returned." **Project-level** credentials come from a downloaded `agc-apiclient-*.json` containing `client_id` and `client_secret`. Roles chosen at creation set permissions. https://developer.huawei.com/consumer/en/doc/app/agc-help-connect-api-obtain-server-auth-0000002271134661

### (d) Authentication

`POST https://{domain}/api/oauth2/v1/token`, `Content-Type: application/json`, body `{"grant_type":"client_credentials","client_id":"…","client_secret":"…"}`. Domain must match the project's data processing location: China `connect-api.cloud.huawei.com`, Germany `connect-api-dre.cloud.huawei.com`, Singapore `connect-api-dra.cloud.huawei.com`, Russia `connect-api-drru.cloud.huawei.com`. Response `access_token`, `expires_in` (the example shows **172800 s = 48 h**; "If the API is called to obtain the value of access_token twice within 48 hours, both access tokens are valid"); on failure `ret:{code,msg}`. Publishing calls send **`client_id: <id>`** and **`Authorization: Bearer <token>`** headers (OAuth-client mode instead sends `teamId` and `oauth2Token`). Header names are from the app-file-info and app-submit pages above.

### (e) Publishing API (`https://connect-api.cloud.huawei.com/api/publish/v2/...`; use the regional domain for a regional project)

| Purpose | Request | Notes and source |
|---|---|---|
| Get upload URL | `GET /api/publish/v2/upload-url?appId=…&suffix=apk\|aab\|app…&releaseType=…` (newer flow also `contentLength`, SHA-256) | Returns `uploadUrl` and `authCode` (older flow) or `uploadUrl` plus headers (newer: `x-amz-content-sha256`, `Authorization`, `x-amz-date`, `Content-Type` must be echoed; file size must equal `contentLength`; extension must match). (secondary: https://medium.com/@kacarselimcan9/huawei-appgallery-uploading-apk-aab-via-api-62a3c1a43653). Official page 503: UNVERIFIED field list |
| Upload file | older: `POST <uploadUrl>` multipart form, part `file`, plus form fields `authCode`, `fileCount`; newer: upload to the returned URL with the returned headers | Chunk upload URLs from `/api/publish/v2/upload/multipart/parts`, valid **4 minutes**; one source says 4 GB single-upload limit (secondary) |
| Attach package or assets | `PUT /api/publish/v2/app-file-info?appId=…[&releaseType=1\|3]`, body `{"fileType":5,"files":{"fileName":"X.apk","fileDestUrl":"<objectId>"}}` | `fileType`: 0 icon, 1 intro videos+posters (up to 3 pairs, video before poster), 2 screenshots (needs `lang`, `imgShowType` 0 portrait / 1 landscape), 3 promo video+poster, 4 promo image, **5 app package (RPK, APK, AAB)**, 6 agent certificate, 7 PDF certificate ≤ 5 MB, 8 and 9 China licences, 10 to 16 VR assets. `deviceType` 4 phone, 6 VR, 7 watch, 8 Vision, 9 router, 10 telematics. `publishCountry` must be non-empty first. Response `{"ret":{"code":0,"msg":"success"},"pkgVersion":[]}`. Official: app-file-info page. The page lists no `.app`/HarmonyOS value; HarmonyOS `.app` through `fileType` 5 is an inference from the Fastforge tool and a forum summary (secondary) |
| Update basic info | `PUT /api/publish/v2/app-info?appId=…` | endpoint named in a Huawei reference page title ("agcapi-app-info_update_v2"); body UNVERIFIED |
| Update listing text | `PUT /api/publish/v2/app-language-info` (`lang` mandatory; `appName` mandatory for a new language; `appDesc` ≤ 8000, `briefInfo` ≤ 80, `newFeatures` ≤ 500; `appId`, `releaseType` as query) | https://developer.huawei.com/consumer/en/doc/development/AppGallery-connect-References/agcapi-language-info-update-0000001158245057 (secondary summary) |
| Check state | `GET /api/publish/v2/app-info?appId=…&lang=…&releaseType=…` | returns `releaseState` (Integer), `auditInfo` (review comments), `phasedReleaseInfo` (`phasedReleaseStartTime/EndTime/Percent/Description`), `defaultLang`, `appName`. Value table for `releaseState`: UNVERIFIED |
| Submit | `POST /api/publish/v2/app-submit?appId=…[&releaseTime=yyyy-MM-ddTHH:mm:ssZZ][&remark=10..300 chars][&releaseType=1\|3]` | `releaseType` 1 whole network (default), 3 by phase (needs an already released version); body with `phasedReleaseStartTime`, `phasedReleaseEndTime`, `phasedReleasePercent`, `phasedReleaseDescription` mandatory when 3; other optional body fields `state`, `isPureDetection`, `sensitivePermissionIconUrl`, `isCommitSensitivePermissionTips`. Response `{"ret":{"code":0,"msg":"success"}}`. Wait ~2 minutes after a package upload before submitting; packages are parsed asynchronously (secondary: Medium/Azure guides) |
| Phased release control | a `phased-release-update` endpoint with `state` SUSPEND or RELEASE | listed by the community reference https://agccli.app/en/reference/publishing/ only: UNVERIFIED |

- **Result codes:** `ret.code == 0` is success. The full code table is on a separate "Result Codes" page that I could not read: UNVERIFIED. Known HTTP/behaviour: wrong client type gives code 403 (above).
- Also "Submitting an App for Release in Download Mode" (`agcapi-app-submit-with-file_v2`) exists for submitting with a file URL: not read.

### HarmonyOS / HarmonyOS NEXT specifics

- **Package:** an **App Pack `.app`** built in release mode (`.hap` cannot be uploaded to the console). Atomic services: ≤ 10 MB total, each stage-model module ≤ 2 MB; `.app` for a HarmonyOS app ≤ 4 GB (secondary: forum summary). Official (atomic services, HarmonyOS 4.x-era page): https://developer.huawei.com/consumer/en/doc/app/agc-help-harmonyos-releaseservice-0000001946273965 : "Currently, atomic services can be released only in the Chinese mainland", qualification documents reviewed "within 1 to 3 working days", AGC "will re-sign" the atomic service after release.
- **Signing material (manual, once; secondary https://dev.to/dcdvdv_a_5059c020ba962689/harmonyos-5-a-complete-guide-to-releasing-a-harmonyos-app-or-atomic-service-1hm5):** (1) generate a `.p12` keystore and `.csr` (DevEco `Build > Generate Key and CSR`, or `keytool`); (2) AGC > Certificates, App IDs & Profiles: create a **Release** certificate from the `.csr`, download the **`.cer`**; (3) create a **Release profile** tied to that certificate, download the **`.p7b`**; (4) put the three files and passwords in `signingConfigs` in `build-profile.json5` with fields `type` (HarmonyOS or OpenHarmony), `certpath` (`.cer`), `profile` (`.p7b`), `storeFile` (`.p12`), `signAlg` `SHA256withECDSA`, `keyAlias`, passwords; (5) build the signed `.app` (output like `*-default-signed.app` in `build/output/app/release/`).
- **Is there an API to create certificates or profiles for CI? Not found.** Every source describes console or DevEco only. **UNVERIFIED that none exists.** Limits (secondary): DevEco auto-sign consumes debug-certificate quota (2 debug certificates; later auto-sign fails); an app can have up to 100 profiles.
- **DevEco 26.0.0 and later** (secondary summary of Huawei's release doc): when uploading from DevEco with upload type "AppGallery Connect", the app is **re-signed with cloud-managed certificates and profiles**, so release signing material is not needed locally; earlier versions need the manual flow. Not confirmed on the primary page (unreadable). DevEco upload exists **only in the Chinese mainland** (secondary summary of the AGC docs).
- **Command-line build on Linux or macOS:** Huawei's "Command Line Tools for HarmonyOS" is one zip with the SDK, `hvigorw`, `ohpm`, `hdc`, linter; set `DEVECO_SDK_HOME` and `NODE_HOME` (Node 18 per Servo book), add `hvigor/bin` and SDK toolchains to `PATH`; JDK 17/21/23 for signing. `hvigorw assembleApp --mode project -p product=default -p buildMode=release --no-daemon` builds the App Pack; `assembleHap` builds HAPs; `--pnpm-frozen-lockfile` pins plugins. Manual signing: `java -jar hap-sign-tool.jar sign-app -mode localSign -signAlg SHA256withECDSA -keystoreFile your.p12 … -appCertFile your.cer -profileFile your.p7b -inFile …unsigned.hap -outFile …hap` (`hap-sign-tool.jar` in `sdk/default/openharmony/toolchains/lib/`; OpenHarmony `developtools_hapsigner` also has `sign-profile`). Huawei Cloud suggests 4 vCPU / 16 GB runners (8 GB minimum). Sources (secondary except the first): https://developer.huawei.com/consumer/en/doc/harmonyos-guides/ide-hvigor-commandline (unreadable), https://github.com/openharmony/developtools_hapsigner , https://book.servo.org/building/openharmony.html , https://github.com/onewilk/NSSFT , community Docker image `springtwr/harmonyos-clt`. **Linux build is supported; whether the official CLT is licensed for server/cloud use: UNVERIFIED.**
- **Fastforge** (https://www.harmony-developers.com/p/use-fastforge-to-automatically-build) publishes an `.app` to AppGallery through the Publishing API with `APP_GALLERY_CLIENT_ID` / `APP_GALLERY_CLIENT_SECRET`; it is the best open reference for the HarmonyOS flow.

### China mainland vs international

- Mainland listing needs **ICP filing** (networked apps whose servers are in the mainland; entered under App Release > Version Information > Filing Information), **real-name verification** (enterprise or individual), and (community source) **software copyright registration**; games also a licence (版号) and an extra game-name rule. Offline standalone apps need no ICP (secondary: https://dev.to/moyantianwang/comprehensive-guide-to-publishing-pure-harmonyos-applications-2025-edition-dln and the search summaries above). Official Huawei side-by-side comparison: not found.
- Atomic services: mainland only. DevEco package upload: mainland only. Some parameters (app-type atomic service) vary by developer type.
- Practical consequence (secondary): new Huawei phones in the mainland run HarmonyOS 6 with no Android compatibility, so mainland reach needs a native HarmonyOS app (the existing `leptoskit_platform_harmonyos` ArkUI host) and a Chinese entity.

### (f) By hand

Account registration and verification; creating the **API client**; creating the app record in AGC (name, package name, category); privacy policy and agreements; filing/ICP/copyright/licences (China); content rating; the first **certificate and profile** (HarmonyOS) unless the cloud-managed signing option applies; qualification documents for atomic services.

### (g) Linux

Publishing API: plain HTTPS from Linux. HarmonyOS build: Linux command-line tools exist; DevEco GUI is not needed. No hardware key.

### (h) Rate limits and pitfalls

Rate limits: UNVERIFIED (not found). Pitfalls: team-level client with a project selected returns 403; the `lang` in `app-file-info` must match the app's languages; `publishCountry` must be set first; the Java sample hardcodes `fileType` 0 and `zh-CN`; the sample passes `files` as an object while the table says list; token domain must match the project's data location; assets uploaded before the package must use the right `fileType`.

### (i) Review times

Official: atomic-service qualification review 1 to 3 working days, version review no fixed time. Community (secondary): initial 1 to 3 days, technical validation 2 to 5, content review 1 to 2 days; "reviews can be slow".

---

## 3. Microsoft Store (Windows)

Docs: https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/overview (MSIX CLI, `ms.date` 2026-10-01); https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/overview-exe (MSI/EXE CLI, 2026-10-05); https://learn.microsoft.com/en-us/windows/apps/publish/store-submission-api (MSI/EXE API); https://learn.microsoft.com/en-us/windows/uwp/monetize/create-and-manage-submissions-using-windows-store-services (legacy API for MSIX/UWP, page updated 2026-03-09); https://learn.microsoft.com/en-us/windows/uwp/monetize/manage-app-submissions ; https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/github-actions .

### (b) Account and fees

Since 2025-09-10 individual registration is free via https://storedeveloper.microsoft.com (Windows Developer Blog: https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/). Learn page updated **2026-10-29** says: "With the new onboarding experience, there are **no registration fees** for either account type" (Individual and Company), but only through the new entry point (storedeveloper.microsoft.com); legacy entry points still charge. Individual: MSA sign-in, government ID plus selfie. Company: DUNS number or business documents plus work-email domain match; manual review 2 to 5 business days (documents 3 to 5). Individual cannot be converted to Company. https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/open-a-developer-account

### (c) Artifacts

- **MSIX** (via `msstore package` / `.msix`, `.msixbundle`; PWABuilder also emits `.classic.appxbundle`): MSIX identity must match the reserved product in Partner Center. The Store re-signs accepted packages; whether an unsigned MSIX is accepted for upload: UNVERIFIED in this session.
- **MSI/EXE:** the developer hosts the installer; the CLI and API submit **package metadata containing the installer URL**; "the CLI does not package or upload the installer binary". `packageType` accepts `exe`, `msi`; `architectures` one of `Neutral, X86, X64, Arm, Arm64`; plus `languages`. Hosting requirements: https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/upload-app-packages (not read).
- Both CLIs: "App update operations … currently supported for free products only." Paid products: later.

### (d) Authentication

Entra ID app registration added in Partner Center: **Account settings > User management > Microsoft Entra applications**, assigned the **Manager** role; record tenant ID, client ID, and a client secret (≤ 24 months, Microsoft recommends < 12). Needs an Entra ID tenant (can be created in Partner Center) and Global administrator for setup.

- **MSI/EXE API:** `POST https://login.microsoftonline.com/<tenant-id>/oauth2/v2.0/token`, `application/x-www-form-urlencoded`, `grant_type=client_credentials&client_id=…&client_secret=…&scope=https://api.store.microsoft.com/.default`. Token **60 minutes**. Calls to **`https://api.store.microsoft.com`** with `Authorization: Bearer <token>` and `X-Seller-Account-Id: <Seller ID>`.
- **MSIX/UWP legacy API:** `POST https://login.microsoftonline.com/<tenant_id>/oauth2/token` with `resource=https://manage.devcenter.microsoft.com`; token 60 minutes; base `https://manage.devcenter.microsoft.com/v1.0/my/`.
- **Secretless option (community):** exchange a GitHub OIDC token for the Entra token with a federated credential (https://github.com/akivajp/msstore-submit); useful because the hosted service can hold no client secret. Not an official Microsoft page.

### (e) Endpoints

**MSI/EXE API** (all under `https://api.store.microsoft.com`, from the store-submission-api page):

| Purpose | Method and path |
|---|---|
| Draft metadata | `GET`/`PUT`/`PATCH /submission/v1/product/{productId}/metadata` |
| Draft packages | `GET /submission/v1/product/{productId}/packages`, `GET …/packages/{packageId}`, `PUT …/packages`, `PATCH …/packages/{packageId}` |
| Commit packages | `POST /submission/v1/product/{productId}/packages/commit` |
| Listing assets | `GET /submission/v1/product/{productId}/listings/assets?languages={languages}`, `POST …/listings/assets/create` (returns a SAS URL; `PUT` to it with `x-ms-blob-type: BlockBlob`, `Content-Type: image/png`), `PUT …/listings/assets/commit` |
| Readiness | `GET /submission/v1/product/{productId}/status` (poll `isReady`; error code `packageuploaderror` means commit again) |
| Submit | `POST /submission/v1/product/{productId}/submit` |
| Status | `GET /submission/v1/product/{productId}/submission/{submissionId}/status` (`publishingStatus` is `INPROGRESS`, `PUBLISHED`, `FAILED`, `UNKNOWN`; `hasFailed`) |

Envelope `isSuccess`, `errors[{code,message,target}]`; each response carries `X-Correlation-ID`; `Retry-After` on throttling; `ongoingSubmissionId` + `pollingUrl` when a submission is in progress. Metadata limits include `description` 10 000 chars and `searchTerms` ≤ 7 terms.

**MSIX/UWP legacy submission API** (`https://manage.devcenter.microsoft.com/v1.0/my/applications/{applicationId}/…`, from manage-app-submissions): `GET …/submissions/{submissionId}`, `GET …/submissions/{submissionId}/status`, `POST …/submissions`, `PUT …/submissions/{submissionId}`, `POST …/submissions/{submissionId}/commit`, `DELETE …/submissions/{submissionId}`; **gradual rollout**: `GET …/packagerollout`, `POST …/updatepackagerolloutpercentage`, `POST …/haltpackagerollout`, `POST …/finalizepackagerollout`; enable by setting `packageRollout.isPackageRollout=true` and `packageRolloutPercentage` on the submission. **Package flights** have separate flight and flight-submission methods (manage-flights, manage-flight-submissions; paths not read). Warnings: after creating a submission by API, change it only by API; API returns 409 for apps using mandatory updates or Store-managed consumables; Pricing Version 2 apps give unknown pricing tier. The page does not say this legacy API is deprecated; the CLI `msstore` MSIX commands (`submission status|get|getListingAssets|updateMetadata|update|poll|publish|delete|rollout`, `flights list|get|delete|create|submission`, `init`, `package`, `publish`) sit on top of Partner Center APIs.

`msstore` CLI: Windows 10+, **Linux, macOS**; CI auth with tenant ID, client ID, client secret, seller ID (`msstore reconfigure --tenantId … --clientId … --clientSecret …`).

### (f) By hand

Account; reserving the app name; the **first submission through Partner Center, including the age-ratings questionnaire and certification**, before any API call; listing, pricing, privacy policy, store-listing screenshots for the first submission; creating the Entra app and giving it Manager.

### (g) Linux

The MSI/EXE API and `msstore` CLI run on Linux. Building a signed MSIX normally needs Windows tooling (`makeappx`/`signtool`, `msstore package`): UNVERIFIED whether a Linux-built MSIX is accepted; leptoskit's Windows backend is Win32, so the packaging step belongs on a Windows build machine.

### (h) and (i) Limits and review

Rate limits: `Retry-After` header, no figures. Publishing "includes scanning and certification and can take more than a day"; secondary sources: "most reviewed in 24 to 48 hours" (https://www.mobiloud.com/blog/publishing-pwa-app-store/) UNVERIFIED.

PWA: PWABuilder packages a PWA to `.msixbundle`/`.classic.appxbundle` from the URL and Partner Center IDs (secondary: https://progressivewebapps.com/installation/app-stores/).

---

## 4. Mac App Store (distinct from Developer ID)

Sources: https://help.apple.com/asc/appsaltool/en.lproj/static.html ; https://developer.apple.com/help/app-store-connect/manage-builds/upload-builds ; https://developer.apple.com/help/account/certificates/certificates-overview/ ; secondary https://blog.xojo.com/2025/01/14/uploading-macos-builds-to-app-store-connect/ , https://developer.apple.com/forums/thread/746354 .

- **Sandbox:** `com.apple.security.app-sandbox` is required for the Mac App Store and not for Developer ID (secondary: Xojo, Apple forums). The plan already records that the macOS host is not sandboxed today, so the Mac App Store is blocked on that.
- **Signing certificates** (Apple certificates overview): *Mac App Distribution* signs the app, *Mac Installer Distribution* signs the installer package (older names "3rd Party Mac Developer Application/Installer"). Apple: "Distribution certificates belong to the team and only one type of each distribution certificate (with the exception of Developer ID certificates) is allowed per team. Only the Account Holder or Admin role can create distribution certificates."
- **Package:** `productbuild --component My.app /Applications Product.pkg --sign "<installer identity>"` (or `productsign`); the App Store takes only the simple pkg (installs one app in `/Applications`) (secondary). A provisioning profile is embedded for the sandbox entitlements.
- **Upload options** (Apple help page): Xcode, Transporter, `altool`, the **App Store Connect API**, Xcode Cloud. `xcodebuild -exportArchive` with `method` `app-store-connect` and `destination` `upload` also works (already in the plan for iOS). `altool` syntax (Apple): `xcrun altool --upload-package <pkg> --apple-id <id> --bundle-id <id> --bundle-short-version-string <v> --bundle-version <n> -t osx` plus **`--apiKey <key> --apiIssuer <issuer>`** (`$API_PRIVATE_KEYS_DIR` points at the `.p8`); `--upload-app` is deprecated ("Use the `--upload-package` command instead"); platform `-t` is `osx`, `ios` or `appletvos`. `altool` notarization stopped **2023-11-01**; the Mac App Store needs no notarization (secondary). A forum report says `altool --upload-app` once silently corrupted a pkg while Transporter worked (secondary).
- **Build Uploads API** (WWDC25 session 324, https://developer.apple.com/videos/play/wwdc2025/324/): `POST /v1/buildUploads` (version, build number, platform), `POST /v1/buildUploadFiles` (returns upload operations: URL, headers, parts), `PUT` the binary parts, `PATCH /v1/buildUploadFiles/{id}` with `uploaded: true`, then processing; webhook type `buildUploads`. Valid `assetType`: `ASSET`, `ASSET_DESCRIPTION`, `ASSET_SPI` (not `IPA`) (secondary: dev.to article). **Whether `platform: MAC_OS` and `.pkg` are accepted: UNVERIFIED** (Apple's reference page for "Create a Build Upload" returned 404). This is the only upload path usable from Linux with just an API key.
- **App Store Connect API differences from iOS:** same endpoints; the app version carries `platform` `MAC_OS`; category and screenshot display types differ (macOS screenshot sizes); App Review needs a Mac build processed. Exact `appStoreVersions`/screenshot-set enums for macOS: UNVERIFIED in this session (the plan already checked iOS `ScreenshotDisplayType` values on the developer documentation JSON).
- **Linux?** The pkg build and signing (`productbuild`, `codesign`) need **macOS**; the upload alone can be done from Linux via the Build Uploads API.
- Review: Apple App Review applies (same as iOS; times not re-researched).

---

## 5. Samsung Galaxy Store (Content Publish API)

Docs: https://developer.samsung.com/galaxy-store/galaxy-store-developer-api/content-publish-api.html ; https://developer.samsung.com/galaxy-store/galaxy-store-developer-api/create-an-access-token.html ; https://developer.samsung.com/galaxy-store/galaxy-store-developer-api/use-the-access-token.html ; staged rollout blog https://developer.samsung.com/galaxy-store/blog/en/2026/05/28/managing-staged-rollouts-using-the-content-publish-api-with-galaxy-store-seller-portal ; failure codes page `…/content-publish-api/failure-response-codes.html` (not read); reference `…/reference.html` (not read).

- **(b) Account/fees:** Seller Portal account (https://seller.samsungapps.com); registration fee and verification: UNVERIFIED (not found on pages read). The API "doesn't support new app registration": apps must already be registered in Seller Portal.
- **(d) Auth:** In Seller Portal > Assistance > API Service create a **service account** (gives a private key and a service-account ID). Build a JWT, **RS256**, header `{"alg":"RS256","typ":"JWT"}`, claims `iss` = service account ID, `scopes` (e.g. `publishing`, `gss`), `iat`, `exp` (≤ **20 minutes** after `iat`). `POST https://devapi.samsungapps.com/auth/accessToken` with `Content-Type: application/json` and `Authorization: Bearer <jwt>`. Response `{"ok":true,"createdItem":{"accessToken":"…"}}`. The doc says "**The access token never expires**, but it can be revoked or cancelled." Errors: 401 `AUTH_REQUIRE`, 403 `NO_PERMISSION`, 422 `REQUEST_VALIDATION_FAILED`. Every API call sends `Authorization: Bearer <accessToken>` and `service-account-id: <id>`; `Content-Type: application/json` except `fileUpload` (multipart/form-data). HTTPS only. A never-expiring token is a long-lived secret: a hosted vault should mint a fresh one and revoke it, or hold the private key and mint per job.
- **(e) Endpoints** (base `https://devapi.samsungapps.com/seller/…`; the file-upload host differs, `https://seller.samsungapps.com/galaxyapi/fileUpload`; posting a file to the `devapi` host gives HTTP 413, forum report):

| Purpose | Method and path |
|---|---|
| List / read apps | `GET /seller/contentList`, `GET /seller/contentInfo?contentId=…` (gives `binarySeq` values) |
| Create upload session | `POST /seller/createUploadSessionId` |
| Upload file | `POST /galaxyapi/fileUpload` (`multipart/form-data`, session ID; field list UNVERIFIED, on the reference page) |
| Modify app data (images, icons, binary info) | `POST /seller/contentUpdate` |
| Binaries (v2) | `POST` / `PUT` / `DELETE /seller/v2/content/binary` |
| Submit for review | `POST /seller/contentSubmit` |
| Change status (e.g. release a manual publication) | `POST /seller/contentStatusUpdate` with `contentStatus: "FOR_SALE"` |
| Closed beta | `GET`/`PUT /seller/v2/content/betaTest` |
| Staged rollout rate | `GET /seller/v2/content/stagedRolloutRate?contentId=…&appStatus=…`, `PUT /seller/v2/content/stagedRolloutRate` (`contentId`, `function` `ENABLE_ROLLOUT`|`DISABLE_ROLLOUT`, `appStatus`, `rolloutRate`, optional `countries:[{countryCode,rolloutRate}]`) |
| Staged rollout binaries | `GET`/`PUT /seller/v2/content/stagedRolloutBinary` (`function` `ADD`|`REMOVE`, `binarySeq`) |

  Flow (Samsung Python walkthrough, https://developer.samsung.com/sdp/blog/en/2022/06/02/app-management-using-the-galaxy-store-developer-api-and-python): token, `contentInfo`, `createUploadSessionId`, `fileUpload`, `contentUpdate` with the new binary info, `contentSubmit`. Staged rollout constraints: rate cannot decrease for live apps; one rollout per content ID; binary updates only in `REGISTERING`, `UPDATING`, `RE_REGISTERING`, `READY_FOR_SALE`; disabling is permanent; "Staged rollouts don't bypass Samsung's review process."
- **Review status:** `contentInfo` returns the content status; field list UNVERIFIED. **(i) Review time:** UNVERIFIED.
- **(f) By hand:** seller account, registering the app (first listing), content rating, service account creation, agreements and tax.
- **(g) Linux:** yes (plain HTTPS). **(h) Limits:** not stated on pages read.

---

## 6. Amazon Appstore (App Submission API)

Docs: https://developer.amazon.com/docs/app-submission-api/overview.html ; auth https://developer.amazon.com/docs/app-submission-api/auth.html ; flows https://developer.amazon.com/docs/app-submission-api/flows.html ; blog with curl https://developer.amazon.com/apps-and-games/blogs/2025/02/using-the-amazon-app-submission-api .

- **(b):** Amazon Developer Console account; fee: UNVERIFIED. The API is "for Android apps only" and **does not support Android App Bundles**: "The App Submission API doesn't currently support Android App Bundles (AAB)." (overview). APKs only (APK > 300 MB use `uploadLargeAPK` then `attachAPK`).
- **(d) Auth (Login with Amazon, client credentials):** create a security profile and attach it under Apps & Services > API Access. `POST https://api.amazon.com/auth/o2/token`, form fields `grant_type=client_credentials`, `client_id`, `client_secret`, `scope=appstore::apps:readwrite` (the doc page says `Content-Type: application/x-www-form-urlencoded`; the blog shows JSON: either may work, UNVERIFIED which is canonical). Response `access_token` (begins `Atc|`), `expires_in` **3600**, `token_type` `bearer`. Errors: `invalid_client`, `invalid_scope`, `unsupported_grant_type`, `invalid_request`; 403 "Request is not authorized." after expiry.
- **(e) Endpoints** (`API_ROOT_PATH` = `https://developer.amazon.com/api/appstore/v1/applications/{app_id}`, from the earlier search summary of the same docs; the blog uses `$API_ROOT_PATH`): `POST $API_ROOT_PATH/edits` (create edit), `GET …/edits` (active edit; `getPreviousEdit` returns null if none), `POST …/edits/{edit_id}/apks/upload` (headers `Content-type: application/octet-stream`, `fileName: <name>.apk`, body binary), `PUT …/edits/{edit_id}/apks/{apk_id}/replace` (keeps device targeting; needs `If-Match: <ETag>`), `PUT …/edits/{edit_id}/listings/{lang}` (e.g. `en-US`; `If-Match`), `POST …/edits/{edit_id}/listings/{lang}/{imageType}/upload` (image types: `small-icons`, `large-icons`, `screenshots`, `promo-images`, `firetv-screenshots`, `firetv-icons`, `firetv-backgrounds`, `firetv-featured-backgrounds`, `firetv-featured-logos`; single-image types are replaced, multi-image types append), `PUT …/availability` (`dateTime` `YYYY-MM-DDThh:mm:ss.s`, `zoneId`), **`POST …/edits/{edit_id}/commit`** with `If-Match: <ETag of the edit>`; after commit the edit status becomes `SUBMITTED` and "the review and publishing procedure will begin immediately by default". Updates are **PUT** only ("HTTP PATCH is not supported").
- **Validate:** no separate validation call; `commitEdit` validates ("submits the Edit if there are no validation errors"). Edits expose `details` and `targeting` resources (`modifyDetails`, `modifyTargeting`).
- **Constraints:** one open edit per app; the first version must go through the console; content rating, availability, pricing, DRM stay console-only; no LAT version through the API; avoid editing one edit from both API and console (an HTTP 412 on update can mean the app is in review).
- **Review status:** no status API described; use the Developer Console App Status screen. Rate limits and review times: not stated on the pages read.
- **Fire OS / Vega OS:** the blog scopes to Fire TV and Fire tablets; **Vega OS is not mentioned** in either page. Vega (React Native-based Fire TV OS): UNVERIFIED, no docs read.
- **(g) Linux:** yes.

---

## 7. Meta Horizon Store (Quest)

Docs: https://developers.meta.com/horizon/resources/publish-reference-platform-command-line-utility/ ; lifecycle https://developers.meta.com/horizon/resources/publish-app-review/ ; submit https://developers.meta.com/vr/resources/publish-submit/ ; device targeting https://developers.meta.com/vr/resources/publish-release-channels-device-targeting/ ; GitHub Actions (secondary) https://github.com/buildalon/upload-meta-quest-build .

- **(b):** Meta developer account with a registered organisation and app on the Developer Dashboard. Fee: UNVERIFIED.
- **(c):** signed Android **APK** (optionally OBB, asset files, language packs, debug symbols). The release must be signed with an Android certificate before submission.
- **(d) Auth:** `ovr-platform-util` with **App ID plus App Secret** ("App Credentials", API tab of the Developer Dashboard; "Treat your App Credentials like a password") or a **user token** (Account settings, "Get Oculus platform utility token"; `get-access-token` prints one). Uploads accept either; `download-quest-build` and `download-asset-file` require a token.
- **(e) Commands:** `ovr-platform-util upload-quest-build -a <APP_ID> -s <APP_SECRET> --apk <file.apk> -c <channel> [--obb <file>] [-n "<notes>"] [--age-group TEENS_AND_ADULTS|MIXED_AGES|CHILDREN] [--assets-dir …] [--asset-files-config …] [--language-packs-dir …] [--debug-symbols-dir …] [--draft] [--clean] [--config …] [--inherit-asset-files] [--exclude-addons]`. "If an intended age group is not specified, the upload will go into 'draft' status" and a draft "can be later published on the Developer Dashboard". `get-release-channel-data` lists channels (name, ID, latest version). `set-release-channel-build --source-channel <a> --destination-channel <b>` copies a build between channels. Channel names are case-insensitive; common: ALPHA, BETA, RC, LIVE; **`store`** is Production ("Production (Store)"). Device targeting is a suffix: `--channel "alpha:quest2only|questproonly"`.
- **Channels:** Alpha, Beta, Release Candidate give external testers access; **Production** is for Meta review. Uploading to Production does not submit it: you "upload a build to Production and submit it from the Developer Dashboard" (https://developers.meta.com/horizon/resources/publish-app-review/). Once live, a new binary uploaded to Production "goes to all existing users immediately, without review" (secondary summary of Meta's lifecycle page: re-verify before relying on it). **There is no documented submit-for-review API**: submission, the Data Use Checkup, the listing and age rating are dashboard steps.
- **(g) Linux:** "The utility runs on Windows, macOS, and Linux", Linux labelled **experimental**; download URL for Linux: `https://www.oculus.com/download_app/?id=5159709737372459&access_token=OC%7C5159709737372459%7C` (Meta page), then `chmod +x`. Codemagic's docs run it on a Linux CI host.
- **(i) Review:** Technical review (VRCs) then Content review; "several business days", no firm SLA; submit ≥ 2 weeks before launch; Data Use Checkup typically 2 business days; metadata updates 1 to 2 business days; app locked while under review (secondary summaries of Meta's pages).
- **(f) By hand:** org and app creation, store listing and assets, age group, Data Use Checkup, VRC compliance, submission, release scheduling.

---

## 8. Chinese Android stores (brief)

| Store | Documented upload API? | Auth | Notes |
|---|---|---|---|
| **vivo** | Yes: vivo Open API (https://developer.vivo.com/doc/detail?id=73 ; PDF) , RESTful, `https://developer-api.vivo.com/router/rest`, methods `app.upload.apk`, `app.upload.obb`, `app.upload.image`, `app.upload.icon`, `app.upload.screenshot`, publish real-time or scheduled | access key assigned after enabling API management, requests signed over the parameters (HMAC per a third-party reference; `access_key`/`access_secret`) | search summary; signature algorithm UNVERIFIED |
| **OPPO** | Yes (open.oppomobile.com publishing API; I only reached an ad-alliance page, https://open.oppomobile.com/new/developmentDoc/info?id=12971) | `client_id` + `client_secret` -> `access_token` (`client_credentials`), plus `X-Api-Sign` over token+timestamp+nonce (secondary) | API reportedly handles updates only; first publication manual (secondary) |
| **Honor** (AppMarket Connect) | Likely; official page not found | `client_id` + secret from Management Center > Credentials (secondary) | token endpoint UNVERIFIED |
| **Xiaomi** | Console flow documented (https://global.developer.mi.com/document?doc=appManagement.createAndUpdate; APK name must match the created package); API: UNVERIFIED | | |
| **Tencent MyApp (应用宝)** | UNVERIFIED (no source read) | | |

All require Chinese real-name and (mainland) ICP/copyright documents. A multi-store reference implementation exists (third-party, unverified accuracy): https://github.com/junbin-lai/app-publish-hub . Note: Oppo, Vivo, Xiaomi and Honor rolled out **Android Developer Verification in China** around 2026-11-09 for sideloaded apps (https://www.gizmochina.com/2026/11/09/oppo-vivo-xiaomi-and-honor-launch-android-developer-verification-adv-in-china/), unrelated to store APIs but relevant to signing identities. Rate limits and review times: UNVERIFIED.

---

## 9. Linux

### Flathub
- **Submission is a pull request**, not an API: fork `flathub/flathub` with "Copy the master branch only" unchecked, check out **`new-pr`**, branch from it, add the manifest, run `flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest <manifest>`, open the PR **against `new-pr`** titled "Add org.example.MyAwesomeApp" (a bot closes PRs against `master`). Comment `bot, build` to run Flathub CI. Explain permissions in the PR. Enable 2FA and accept the repository invitation within one week. "There is no definite time limit as all reviewers are volunteers"; merges in batches; excessive AI-generated content may be closed. https://docs.flathub.org/docs/for-app-authors/submission
- **Verification:** website verification file at `https://<site>/.well-known/org.flathub.VerifiedApps.txt` with a token from the Flathub Developer Portal (secondary: https://docs.pears.com/how-to/operate-an-app/build-and-package/submit-to-app-stores/). App ID rules are on the Requirements page (not read).
- **Updates** go to the per-app repository the maintainer gets access to after acceptance; Flathub CI builds. **Direct uploads** (build on your own CI, push an OSTree repo with `flat-manager-client`) are granted selectively to verified, trusted upstreams; request via an issue labelled "direct-uploads" on `flathub/flathub`; the manifest and pipeline must be public; linter rules: `--compose-url-policy=full --mirror-screenshots-url=https://dl.flathub.org/media`, MetaInfo `flathub::manifest` tag, stable and beta refs to the right repos (secondary: Flathub linter page https://docs.flathub.org/docs/for-app-authors/linter and issue threads). Flathub API token for direct uploads: **UNVERIFIED** how issued (Developer Portal per the 2023 plan, secondary).
- Automation level: **manual PR** (first submission); updates via the app's own repo. Linux server: yes (git + `flatpak-builder` needs Linux).

### Snap Store
- `snapcraft export-login [--snaps a,b] [--channels stable] [--acls package_access,package_push,package_update,package_release] [--expires <ISO-8601>] <file|->` writes credentials; export as **`SNAPCRAFT_STORE_CREDENTIALS`** (fails if the file exists; `snapcraft login` fails while the variable is set). `snapcraft upload app_1.0_amd64.snap --release=edge` uploads and releases; `snapcraft release <name> <revision> <channel>[,<channel>…]` releases an existing revision; tracks as `1.0/stable`. GitHub action `snapcore/action-publish` (`release` input omitted means upload without release). Sources: https://documentation.ubuntu.com/snapcraft/latest/how-to/publishing/authenticate/ , https://ubuntu.com/docs/snapcraft/latest/reference/commands/export-login/ , https://github.com/canonical/action-publish (secondary summaries).
- Account: Snap Store developer account (free: UNVERIFIED); the snap name must be registered first (`snapcraft register`, by hand). Store review for new snap names/classic confinement: manual, times UNVERIFIED. REST API (`dashboard.snapcraft.io` macaroon API): endpoints **UNVERIFIED**; use the `snapcraft` CLI, which runs on Linux only.
- Automation level: **full CLI/API** after the first registration. Runs on **Linux**.

### AppImage
No store. AppImageHub is a catalogue fed by a GitHub-topic or PR flow: UNVERIFIED (not researched). Distribution = host the file (GitHub Releases) with `zsync` update info. Automation: none needed beyond a release upload.

## 10. Open and alternative Android distribution, and iOS alternatives

| Channel | Mechanism | API? | Source |
|---|---|---|---|
| **F-Droid** (main repo) | Merge request to `gitlab.com/fdroid/fdroiddata` adding `metadata/<applicationId>.yml`; templates in `templates/`; check with `fdroid import`, `fdroid rewritemeta`, `fdroid lint`, `fdroid checkupdates`, `fdroid build -v -t -l <id>`; F-Droid's builder builds from source and signs with its own key (or publishes the developer's signed APK if the build is **reproducible**, via a `Binaries:` URL); the builder then auto-bumps versions ("F-Droid Builder" commits) | None: manual MR; fully automated afterwards | https://f-droid.org/docs/Inclusion_How-To/ (via mirror https://cloudflare.f-droid.org/en/docs/Inclusion_How-To/), https://f-droid.org/docs/FAQ_-_App_Developers/ |
| **Self-hosted F-Droid repo** | `fdroidserver`: `fdroid init`, put APKs in `repo/`, `fdroid update`, serve static files; simple binary repo or full build; Linux | No remote API; local tool | f-droid.org docs above; step details UNVERIFIED (not read) |
| **Aptoide** | Aptoide Connect: first version via console form; later versions via **Uploader API** `POST https://uploader.catappult.io/api` with an API key (needs an active Aptoide Connect subscription); `.apk` ≤ 4 GB, `.obb` ≤ 5 GB, signed APK, no AAB; HTTP 200 then awaits Aptoide approval | **Yes** | https://docs.connect.aptoide.com/apis/android-app-version-submission-api , https://aptoide.zendesk.com/hc/en-us/articles/15856508923028-Upload-Your-Android-App-to-Aptoide (search summary; request body UNVERIFIED) |
| **Uptodown** | Developers Console (free): upload or claim authorship, manual review, signature must match the original; `Disable uploads by Uptodown editors` option | **No public upload API found** | https://support.uptodown.com/hc/en-us/articles/4424141383181-Basic-guide-to-register-and-publish-apps-on-Uptodown |
| **APKPure** | Developer Console at developer.apkpure.com; APK/XAP ≤ 2 GB; every APK manually reviewed | **No API found** | https://apkpure.com/howto/how-to-create-apkpure-developer-console-account-for-free (≈720 days old) |
| **Obtainium** | The user's app tracks a public repo's GitHub Releases; same package ID and signing identity every release; pre-releases hidden by default; use an APK-filter regex if several ABIs; no token needed for public repos (rate limit for many apps) | GitHub Releases API (`gh release create`) | https://github.com/ImranR98/Obtainium , https://wiki.obtainium.imranr.dev/sources/ |
| **AltStore PAL** (EU, Japan, Brazil) | Apple Developer Program membership ($99/yr); account holder accepts the updated agreement; authorise AltStore as a marketplace in App Store Connect (Users and Access > Integrations > Marketplace); **submit for Notarization** in App Store Connect (review "with less stringent requirements than the App Store"); download the **Alternative Distribution Package (ADP)** through AltStore's REST API (https://faq.altstore.io/developers/rest-api, **not read: endpoints and auth UNVERIFIED**) and host it yourself; publish a **source JSON** (PAL source needs `marketplaceID`; `downloadURL` points at the ADP `manifest.json`); optional listing on explore.alt.store. Fees: AltStore 0% commission; Apple 5% Core Technology Commission (per the AltStore developer page) | **Yes** (REST for ADP; notarization via App Store Connect) | https://developer.altstore.io/ ; example source (secondary) https://github.com/brighthills/altstore |
| **Epic Games Store mobile** | Android and iOS (iOS only in the EU) launched August 2024 (Wikipedia, secondary). Developer publishing process: **UNVERIFIED**, no official page found | UNVERIFIED | |
| **AltStore Classic / sideload** | no notarisation, worldwide | n/a | AltStore developer page |

Linux: F-Droid, Aptoide, Obtainium (GitHub release) all work from Linux; AltStore PAL needs a notarised iOS build, so the Apple/macOS pipeline stays.

## 11. Games and desktop (brief; repo has `leptoskit_game`)

- **Steam:** Steamworks account; **Steam Direct fee $100 per app, recoupable after $1 000 adjusted gross revenue** (secondary: https://www.steampageanalyzer.com/blog/steam-direct-fee-explained, Valve's own page not read); onboarding paperwork 15 to 30 days (secondary). Upload: `steamcmd +login <build-account> +run_app_build ..\scripts\app_build.vdf +quit`; the app build VDF has `AppID`, `Depots` (file mappings), `SetLive` (beta-branch name only; the **default branch cannot be set live by the script**, only in the App Admin panel). Build account: dedicated, needs "Edit App Metadata" and "Publish App Changes To Steam"; to set live for a released app it needs a phone number or Steam Mobile App, and after security changes (email/phone) wait 3 days. **Steam Guard in CI:** log in once interactively, keep the config file between runs (it holds the login token); a fresh password login issues a new token needing a code; `set_steam_guard_code <code>` for the emailed code. `steamcmd` has a Linux version (`builder_linux`); Linux/macOS depots need adding to a package. https://partner.steamgames.com/doc/sdk/uploading . Hosted-service implication: the saved Steam Guard config is a long-lived credential tied to one machine/session; treat like a secret and keep it per tenant. Automation: **full CLI** (release to default branch is manual or by `SetLive` branches).
- **itch.io:** `butler push <directory|zip> <user>/<game>:<channel>`; channel names set platform tags (`win`, `windows`, `linux`, `mac`, `osx`, `android`, e.g. `win-linux-mac-stable`); `--userversion 1.1.0` or `--userversion-file`; `--if-changed` for CI. API key auth (`BUTLER_API_KEY` or `butler login`): the page I read did not state this; **UNVERIFIED here** (documented on butler's login page). HTML5 "playable in browser" is set by hand after the first push. https://itch.io/docs/butler/pushing.html . No review gate for uploads (game page publication is manual). Linux: yes.
- **Epic Games Store:** `BuildPatchTool` (BPT) in `UploadBinary` mode with `-OrganizationId`, `-ProductId`, `-ArtifactId`, `-ClientId`, `-ClientSecret` (or `-ClientSecretEnvVar`), `-BuildRoot`, `-CloudDir`, `-BuildVersion` (must be unique), `-FileIgnoreList`; credentials from Dev Portal > Product Settings > General > Build Patch Tool Credentials (separate from EOS IDs). Docs say examples are Windows but "similar on macOS and Linux". 12% revenue share (Wikipedia, secondary). https://dev.epicgames.com/docs/epic-games-store/publishing-tools/uploading-binaries/bpt-instructions-170 . Product approval and store page: manual.
- **GOG:** `GOGGalaxyPipelineBuilder` CLI (downloaded from the Developer Portal; access needs the GOG product manager): `GOGGalaxyPipelineBuilder.exe build-game project.json --output "game_repository"` (`--username`, `--password`, `--branch`, `--offline`, `--dump_upload_info`); `publish-build <base_product_id> <build_id> <branch>`; **cannot publish to Master or any public branch from the Pipeline Builder** (GOG staff do that). Invite-only partnership; Windows-first (Linux/macOS availability UNVERIFIED). https://docs.gog.com/pb-commands-and-arguments/ , https://docs.gog.com/pb-publish-build/

## 12. Web / PWA routes (brief)

- **Google Play via Trusted Web Activity:** `npm i -g @bubblewrap/cli`; `bubblewrap init --manifest https://example.com/manifest.json`; `bubblewrap build` produces `.aab` and `.apk`. Host Digital Asset Links at `https://<origin>/.well-known/assetlinks.json`; **with Play App Signing the fingerprint in `assetlinks.json` must be Play's app-signing key, not your upload key**; if wrong, a URL bar shows. New Play apps need an AAB. PWABuilder wraps Bubblewrap and writes a starter `assetlinks.json` with the upload key. Sources (secondary): https://www.thinktecture.com/en/pwa/twa-bubblewrap/ , https://vaadin.com/blog/submitting-a-pwa-to-google-play-store-using-bubblewrap . Upload then goes through the existing Google Play client in leptoskit.
- **Microsoft Store via PWABuilder:** enter the PWA URL and Partner Center IDs, get `.msixbundle`/`.classic.appxbundle`; manifest changes need a new package. Upload via section 3. Source (secondary): https://progressivewebapps.com/installation/app-stores/ . Authoritative docs: docs.pwabuilder.com (not read).

---

# Part B: how cloud services hold store credentials

| Service | What the user gives it | How it stores it | Source |
|---|---|---|---|
| **Expo EAS** (Submit + managed credentials) | ASC API key (`.p8`, key ID, issuer ID) = "the default and recommended" method; Google service account JSON; Android keystore (managed); optional Apple ID/app-specific password ("not recommended"); FCM and APNs keys | Google Cloud encrypts data at rest; "**Credentials are additionally encrypted using KMS**"; "only unencrypted for as long as we need them in memory"; "always encrypted in our databases, message queues". ASC API key and Google service-account key **stay on Expo servers for reuse** and can be removed at any time. App-specific password kept for the submission **plus 24 hours**, then deleted. Apple Developer ID/password "we do not store". `.p8` never returned to the client in GraphQL (secondary: deepwiki) | https://docs.expo.dev/app-signing/security/ (read 2026-11-18) |
| **Codemagic** | ASC API key (key ID, issuer ID, `.p8`) as a team-level integration; code-signing identities (certificate `.p12`, profiles) uploaded or generated and stored in Team settings > Code signing identities; can "Fetch certificate" and profiles from Apple using the key; Google service account | Stores them "securely"; **encryption algorithm not described** in what I read. Builds use `app-store-connect fetch-signing-files $BUNDLE_ID --type … --create`, `keychain initialize`, `keychain add-certificates` with env `APP_STORE_CONNECT_KEY_IDENTIFIER`, `APP_STORE_CONNECT_ISSUER_ID`, `APP_STORE_CONNECT_PRIVATE_KEY`; the certificate private key must still be supplied | https://docs.codemagic.io/yaml-code-signing/signing-ios/ (search summary) |
| **Bitrise** | Team ASC API key as an Apple service connection (one connection per project, i.e. one Apple team per project; other keys as step inputs); iOS `.p12` certificates, profiles, Android keystores uploaded to the Code signing tab | Encrypted storage, delivered to a build machine only when a workflow needs them, installed in a **temporary keychain**; algorithms not named. Automatic provisioning downloads or creates profiles through the API key, renews expired ones, registers devices | https://docs.bitrise.io/en/bitrise-platform/integrations/apple-services-connection/connecting-to-an-apple-service-with-api-key , https://docs.bitrise.io/en/bitrise-ci/code-signing/ios-code-signing/managing-ios-code-signing-files---automatic-provisioning (search summaries) |
| **fastlane match** | not a service: certificates and profiles in a private **git repo, Google Cloud Storage or S3** | each `.p12`/`.mobileprovision` encrypted individually with **OpenSSL AES-256-CBC** keyed by `MATCH_PASSWORD`; access by repo/bucket permissions; GCS and S3 also use server-side encryption; `readonly` mode for CI so it never creates certificates; `match nuke` frees certificate slots; an expired certificate's replacement does not revoke the old one on Apple's portal | https://docs.fastlane.tools/actions/match/ , https://docs.fastlane.tools/actions/match_nuke/ (search summaries) |
| **Ionic Appflow** | Android keystores and iOS certificates + profiles **uploaded by the user** (not generated); passwords; for App Store uploads the legacy **Apple ID + app-specific password** | "securely stores"; Capawesome says an Appflow export holds environment secrets, signing certificates with passwords and store credentials **in plain text** and must be deleted after migration; an Appflow shutdown date of 2027-12-31 is a single third-party claim, UNVERIFIED | https://ionic.io/docs/appflow/package/credentials , https://capawesome.io/docs/cloud/migrations/ionic-appflow/ (search summaries) |
| **Runway** | UNVERIFIED: no Runway page on credential storage found; only a tutorial that stores the ASC key in GitHub encrypted secrets | | https://www.runway.team/blog/how-to-set-up-a-ci-cd-pipeline-for-your-ios-app-fastlane-github-actions |

**Security guidance published (Expo):** decide whether you trust the service with credentials; back up the Android keystore; use the ASC API key not an Apple ID; if the storage model is not acceptable, run builds on your own infrastructure. Compromise analysis (Expo): a stolen ASC API key lets an attacker act in App Store Connect within the key's roles, and with build credentials submit new versions of existing apps but not new apps; a stolen Google service-account key plus the upload keystore can submit new versions of existing apps.

### Apple facts needed for a signing vault

- **API key:** team key (Account Holder/Admin) with roles; `.p8` downloadable **once**; JWT **ES256**, header `kid`, claims `iss` (issuer ID), `iat`, `exp` (≤ 20 minutes), `aud` `appstoreconnect-v1`. leptoskit already implements 20-minute tokens (publishing plan progress, Phase 2.1); the key-types page itself could not be read (fetcher returned only the title), so the role names come from that summary.
- **Certificates, profiles, bundle IDs, devices** through the API: `GET`/`POST /v1/certificates` (body `csrContent`, `certificateType`), `GET`/`POST /v1/profiles`, `GET`/`POST /v1/bundleIds`, `GET`/`POST /v1/devices`; Apple's "Create a Certificate" reference returned **404** on 2026-11-18, so request/response shapes are UNVERIFIED here; the repo's Phase 2 code and fastlane's behaviour are the references.
- **Distribution-certificate limit:** Apple's official wording: "only one type of each distribution certificate (with the exception of Developer ID certificates) is allowed per team" and only Account Holder or Admin can create them (https://developer.apple.com/help/account/certificates/certificates-overview/). Third-party sources quote 2 or 3 for legacy iOS Distribution; Apple's page gives **no numeric cap**. Revoking a certificate invalidates every profile that uses it. Design consequence: **one shared Apple Distribution certificate per Apple team**, created once, never per build; list with `GET /v1/certificates` before `POST`; do not revoke to "retry". Do not let two tenants share one Apple team's certificate private key unless the tenant owns the team.
- **Uploads without a Mac:** the Build Uploads API (`POST /v1/buildUploads`, `POST /v1/buildUploadFiles`, `PUT` parts, `PATCH /v1/buildUploadFiles/{id}`) needs **only a JWT**, so the build machine never needs the `.p8`. `xcodebuild -allowProvisioningUpdates` needs the key file on the Mac (the existing plan), so for macOS runners prefer **pre-created signing assets** over giving the runner the key.

### Google facts

- A **service account JSON** invited in Play Console (existing `google.rs`); the service account itself mints short-lived OAuth access tokens (Google standard: 1 hour; not re-fetched in this session).
- **Play App Signing:** Google holds the app signing key; you sign with an **upload key**. Lost upload key: Play Console > Setup > App integrity > **Request upload key reset** (needs a `.pem` certificate from a new keystore); the new key becomes valid after the time in the confirmation email, typically about 48 hours (2 to 3 business days at worst); until then no uploads; users unaffected. Source: third-party guides (e.g. https://www.webtonative.com/blog/steps-to-request-upload-key-reset), no official page read. Without Play App Signing, a lost key means no more updates (Expo security page).
- A hosted service should therefore **generate the upload keystore itself**, keep only that, and enrol the app in Play App Signing; losing it is recoverable, which a legacy-signed app's key is not.

### Recommended credential-vault design for the hosted publisher

1. **Envelope encryption.** Per-tenant data-encryption key (DEK, AES-256-GCM) per secret version; DEKs wrapped by a key-encryption key in a managed KMS/HSM (cloud KMS), with the tenant ID and secret ID as **AEAD associated data** so a ciphertext cannot be moved between tenants. Never store a plaintext secret or a DEK. EAS does the equivalent: KMS on top of at-rest encryption, decrypted only in memory.
2. **Typed secrets, minimal scope.** Store exactly what each store needs and nothing broader: ASC key (`.p8`, key ID, issuer ID) as a *team key with the least role that works*; Google service-account JSON; upload keystore + passwords; Huawei `client_id`/`client_secret`; Samsung service-account private key; Amazon LWA client; Entra client secret (or federated credential); Meta app secret; Solana portal API key; Steam build-account config; `.p12`/`.p7b` signing material. Prefer secrets the store lets you scope (Snap `export-login --snaps … --channels … --acls … --expires`; ASC roles; Play permissions on one app).
3. **Server mints short-lived tokens; the build job never sees long-lived keys, wherever the store allows it.**
   - Apple: server signs a 20-minute JWT; build job gets only the JWT, used for the Build Uploads API and TestFlight calls.
   - Google: server exchanges the service account for a 1-hour access token (or does the Play upload itself from the artifact the job returns).
   - Microsoft: Entra client-credentials token 60 minutes, or OIDC federation with no stored secret.
   - Amazon: LWA token 1 hour. Huawei: token 48 hours (long; request it per job and treat it as a secret). Samsung: JWT ≤ 20 minutes, but the exchanged access token "never expires": revoke after the job, or keep uploads server-side.
   - **Best pattern: let the build machine only produce an artifact, and let the service do the upload/submit steps.** Everything above except `xcodebuild`, `ovr-platform-util`, `steamcmd` and `snapcraft` is a plain HTTPS client, so the long-lived credential never leaves the vault host.
4. **Credentials that must reach a machine** (Android upload keystore, Apple signing `.p12` + profile, HarmonyOS `.p12`/`.cer`/`.p7b`, Steam login config, `ovr-platform-util` secret, snapcraft export): release only to an **ephemeral, single-tenant build job** (fresh VM or container per job, destroyed afterwards), delivered over a mutually authenticated channel as in-memory or `tmpfs` files (mode 0600, private temp directory as leptoskit already does for `LEPTOSKIT_ANDROID_KEYSTORE_BASE64`), with a **per-job lease** (job ID, tenant, secret versions, expiry minutes) recorded before release. Use Bitrise-style temporary keychains for Apple, deleted at job end.
5. **Never log.** Redacting `Secret` type (already in `secrets.rs`); build-log scrubbing for every released secret value and its base64 and URL-encoded forms; no secrets in environment dumps, process lists, artifacts, caches or crash reports; tool flags that put a secret on the command line (`ovr-platform-util -s`, `altool -p`) must go through a file or stdin; no build cache across tenants (the existing release workflow already refuses caches).
6. **Audit log.** Append-only: who created, rotated, leased, used, revoked each secret; which job and which store call; source IP; no secret bytes. Export to the tenant. Alert on lease without job, unusual store, reads outside a job.
7. **Rotation and revocation.** Versioned secrets; rotate on a schedule that matches the store (Edge keys expire after 72 days per the plan; Entra secrets ≤ 24 months; Samsung token never expires); one-click revoke that (a) disables leases, (b) revokes at the issuer where possible (Apple key in App Store Connect, Samsung token, Snap macaroon, Entra secret), (c) cancels running jobs, (d) records an incident. Keep a per-store "how to rotate at the issuer" runbook and a `person` step for each (the issuer UI needs the account holder).
8. **Tenancy and blast radius.** One KMS key per tenant or per environment; per-tenant Apple team (never reuse the Distribution certificate across unrelated tenants); no cross-tenant jobs on one VM; egress allow-list per job (only that store's hosts); jobs cannot read the vault, only receive leases.
9. **Human-only secrets.** Developer wallets, 2FA, account-holder logins and KYC never go into the vault. For Solana, prefer **wallet-delegated signing** (section 1) and keep only the portal API key server-side; if a keypair must be stored, it is a dedicated low-balance wallet.

---

# Part C: synthesis

## All stores

Automation level: **Full** = API/CLI covers upload and submit after one-time dashboard setup. **Partial** = API covers upload but review submission or other steps are manual. **Manual PR** = submission is a pull request. **None** = no API found.

| Store | Automation | Runs on | Auth | Fee | Priority | Client complexity |
|---|---|---|---|---|---|---|
| **Solana dApp Store** | Partial: CLI/library does ingest, NFT mint, submit for review; first app and KYC manual | Linux (Node); wallet signing may need a browser or hardware wallet if delegated | portal API key (`x-api-key`) + publisher wallet keypair or delegated signature | account free (none documented); ~0.016 SOL per release | **P1** | **L** (tRPC procedures, Solana transactions, attestation; or wrap the Node library: M) |
| **Huawei AppGallery (Android)** | Full for uploads and submit after app creation; first listing manual | Linux | `client_id`/`client_secret` -> bearer token (48 h) | UNVERIFIED (likely free) | **P1** | **M** |
| **Huawei AppGallery (HarmonyOS `.app`)** | Partial: API upload as for Android (fileType for `.app` UNVERIFIED); certs/profiles manual (no API found); build on Linux possible | Linux or macOS build; API on Linux | same as above + `.p12/.cer/.p7b` | UNVERIFIED; China needs ICP/copyright/real-name | **P1** | **L** (signing and mainland rules) |
| Microsoft Store (MSIX, MSI/EXE) | Full after first submission by hand | Linux for API/CLI; Windows for MSIX packaging | Entra client-credentials (60 min) or OIDC | free (new onboarding) | **P2** | **M** |
| Meta Horizon Store | Partial: upload to channels by CLI; submit for review in dashboard | Linux (experimental), macOS, Windows | app ID + secret or user token | UNVERIFIED | **P2** | **S** (wrap `ovr-platform-util`) |
| Samsung Galaxy Store | Full after app registered | Linux | service-account JWT -> non-expiring token | UNVERIFIED | **P2** | **M** (upload host quirk, sessions) |
| Mac App Store | Partial: reuse App Store Connect code; needs sandboxed host and Mac build | macOS (build), Linux (upload via Build Uploads API: UNVERIFIED for pkg) | ASC API key | included in $99 Apple Developer Program | **P2** (blocked by sandbox) | **M** |
| itch.io | Full (`butler push`) | Linux | API key | free | **P2** (game host) | **S** |
| Steam | Partial: `steamcmd` build upload; default-branch release manual | Linux | build account + Steam Guard state | $100/app recoupable | **P2** (game host) | **M** |
| Amazon Appstore | Full for updates (edits); first app manual; **no AAB** | Linux | LWA client credentials (1 h) | UNVERIFIED | P3 | **M** |
| Snap Store | Full CLI after registration | Linux | `SNAPCRAFT_STORE_CREDENTIALS` (scoped macaroon) | UNVERIFIED (believed free) | P3 | **S** |
| Flathub | Manual PR (first), then maintainer repo; direct upload invite-only | Linux | GitHub | free | P3 | **M** (manifest generation) |
| F-Droid / self-hosted repo | Manual MR (main repo); self-hosted full | Linux | git / own signing key | free | P3 | **M** |
| Aptoide | Partial: console for first version, Uploader API after | Linux | API key | needs Aptoide Connect subscription | P3 | **S** |
| AltStore PAL | Partial: notarization in App Store Connect, ADP REST, source JSON | macOS (build), Linux (host) | ASC API key + AltStore | $99 + 5% Apple commission; AltStore 0% | P3 | **M** |
| Epic Games Store | Partial: BPT upload; store approval manual | Linux/macOS/Windows (docs: Windows examples) | client ID/secret | 12% revenue share (secondary) | P3 | **S** |
| GOG | Partial, invite-only; no public-branch publish | Windows-first (UNVERIFIED) | GOG account | revenue share (UNVERIFIED) | P4 | **M** |
| vivo / OPPO / Honor / Xiaomi / Tencent | Partial, China entity needed | Linux | per-store (HMAC / OAuth) | UNVERIFIED | P4 | **L** |
| Google Play TWA / PWABuilder / Microsoft Store PWA | via existing Play and Microsoft clients | Linux | existing | existing | P3 (packaging only) | **S** |
| Uptodown, APKPure | None found (consoles only) | n/a | n/a | free | P4 | none |
| Obtainium / GitHub Releases | Full via `gh release` | Linux | GitHub token | free | P3 (cheap, zero store) | **S** |

## "Upload" and "submit" sequences in the existing plan's vocabulary

Notation: **[agent]** a coding agent can do it in a dashboard, **[person]** needs the account holder (sign-in, 2FA, agreements, payment/tax, legal declarations, KYC, wallet, the decision to release).

### Solana dApp Store (`[publish.solana]`, credentials: `LEPTOSKIT_PUBLISH_SOLANA_API_KEY`, `LEPTOSKIT_PUBLISH_SOLANA_KEYPAIR` or a delegated signer)

- **Checks (dry run):** APK is a release build signed with a key that is not the Play key; `arm64-v8a` present; ≤ 2 GB; `versionCode` is exactly previous + 1 and `versionName` changed; icon 512x512 opaque, banner 1200x600, 4 to 8 previews with identical dimensions, name ≤ 25, short description ≤ 50; Terms and Privacy URLs and website respond over HTTPS; signer holds ≥ 0.016 SOL; "What's New" present (from `store/<locale>/release_notes.md`).
- **Upload:** build the APK with the dedicated dApp Store keystore; `createUploadTarget`, `PUT`, `finalizeUpload`; `createIngestionSession` (source `portalUpload`; idempotency key = hash of the APK); poll `getIngestionSession`; this leaves a draft release.
- **Submit (`--submit`):** `prepareReleaseNftTransaction` -> wallet signs -> `submitSignedTransaction`; `saveReleaseNftData`; `prepareVerifyCollectionTransaction` -> sign -> `submitSignedTransaction` -> `markReleaseCollectionAsVerified`; build attestation (slot, blockhash, request ID) -> wallet signs the message -> `submitToStore` with `whatsNew`, `testingInstructions`, `criticalUpdate`, `isResubmission`. Then report "In Review", expect 1 to 2 business days (3 to 4 for a new app).
- **What to do next:** [person] create the portal account, publisher profile, **pass KYC/KYB**; [person] connect the publisher wallet; [agent] fill the app details form from `store/` (website, support email, Terms/Privacy URLs, package name) and upload icon, banner, previews, then [person] approve the **App NFT mint** transaction; [person] create the portal API key; [person] fund the publishing wallet with ~0.02 SOL; [person] (if wallet-delegated) approve each signature request; [person] decide to release; after rejection, [agent] fix the named cause (unreachable site, broken policy link) and resubmit.
- **Never automatable:** KYC/KYB, wallet custody and funding, the App NFT mint approval in the portal, accepting the Publisher Policy and Developer Agreement.

### Huawei AppGallery, Android (`[publish.appgallery]`)

- **Upload:** token (`POST …/api/oauth2/v1/token`, regional domain); `GET upload-url` (suffix `apk`/`aab`); upload the file; `PUT app-file-info` with `fileType` 5; images with `fileType` 0 and 2 (`lang`, `imgShowType`); `PUT app-language-info` (name, description ≤ 8000, brief ≤ 80, new features ≤ 500).
- **Submit:** `POST app-submit?appId=…` (`releaseType` 1, or 3 with phased body after one release exists; wait ≈ 2 minutes after the package upload), then poll `GET app-info` for `releaseState` and `auditInfo`.
- **What to do next:** [person] register and verify the developer account (real-name for China); [person] create the **API client** (team level, project N/A); [agent] create the app record and listing; [person] declare privacy, content rating, filing information (China); [person] decide to release.

### Huawei AppGallery, HarmonyOS (`[publish.appgallery]` with `harmonyos = true`)

- **Build:** `hvigorw assembleApp --mode project -p product=default -p buildMode=release --no-daemon` on a Linux or macOS runner with the Command Line Tools and a JDK; signing config in `build-profile.json5` reading the `.p12`, `.cer`, `.p7b` from the job's tmpfs.
- **Upload:** same as Android with the `.app` file (**UNVERIFIED** `fileType`: verify against a real response, using Fastforge's behaviour as the reference); mainland-only apps require ICP/qualification data already recorded in the app record.
- **Submit:** `app-submit` as above.
- **What to do next:** [person] generate the `.p12`/`.csr` key (keep the key) and **create the release certificate and profile in AGC** (no API found); [person] complete real-name, ICP filing, software copyright, game licences; [agent] fill localised name and icon (216x216 for atomic services), categories, privacy statement; [person] decide to release.
- **Never automatable (as far as found):** release certificate and profile creation, qualification/filing documents, real-name verification.

### Microsoft Store (`[publish.microsoft_store]`, MSIX and MSI/EXE)

- **Upload (MSI/EXE):** token (`scope=https://api.store.microsoft.com/.default`), `GET …/packages`, `PATCH …/packages/{packageId}` with the new installer URL, version, architecture, `POST …/packages/commit`, poll `…/status` until `isReady`, update metadata and listing assets (`…/listings/assets/create` -> SAS `PUT` -> `…/commit`).
- **Upload (MSIX):** `msstore` or the legacy submission API: create a submission, upload the package zip to the returned SAS URL, `PUT` the submission, `POST …/commit`.
- **Submit:** `POST …/submit` (MSI/EXE), or `commit` (MSIX, optional gradual rollout 0 to 100%); poll `…/submission/{submissionId}/status` for `PUBLISHED`/`FAILED`.
- **What to do next:** [person] create the developer account through storedeveloper.microsoft.com (ID check); [agent] reserve the app name; [person] answer age ratings and complete the **first submission in Partner Center**; [person] create the Entra app and grant **Manager**; [agent] fill listing pages; [person] decide to release.

### Meta Horizon Store (`[publish.quest]`)

- **Upload:** `ovr-platform-util upload-quest-build -a <id> -s <secret> --apk <signed.apk> -c alpha|beta|rc --age-group <…> -n "<notes>"`; `get-release-channel-data` to confirm; optional `set-release-channel-build` to promote alpha to beta.
- **Submit:** upload to the `store` (Production) channel, then **[person] submits for review in the Developer Dashboard** (no API found); no automated release afterwards.
- **What to do next:** [person] create org/app, age rating, Data Use Checkup; [agent] store listing text and images; [person] VRC test results; [person] schedule release.

### Samsung Galaxy Store (`[publish.samsung]`)

- **Upload:** JWT -> `POST /auth/accessToken`; `GET /seller/contentInfo`; `POST /seller/createUploadSessionId`; `POST /galaxyapi/fileUpload`; `POST /seller/contentUpdate`.
- **Submit:** `POST /seller/contentSubmit`; optional `PUT /seller/v2/content/stagedRolloutRate`.
- **What to do next:** [person] create the seller account, **register the app once**, create the service account; [agent] listing text and screenshots; [person] decide to release.

### Mac App Store (later, after the sandbox decision)

- **Build:** sandboxed `.app` signed with Mac App Distribution, `productbuild` pkg signed with Mac Installer Distribution (macOS runner).
- **Upload:** `xcrun altool --upload-package … --apiKey … --apiIssuer …` or the Build Uploads API (UNVERIFIED for `.pkg`); then App Store Connect API calls as for iOS with `platform` macOS.
- **Submit:** review submission as in the existing App Store module.
- **What to do next:** the same App Store Connect person steps as iOS (agreements, export compliance, app privacy), plus [person] create the Mac App Distribution and Installer certificates (Admin/Account Holder).

### P2 games: itch.io and Steam

- **itch.io:** [person] create the project page; upload `butler push dist/ user/game:win-linux-mac-stable --userversion <app.version>`; [person] publish the page.
- **Steam:** [person] pay Steam Direct, complete paperwork, create the app and a dedicated build account with a phone attached; [person] one interactive `steamcmd` login to seed the Steam Guard state; upload with `+run_app_build`; `SetLive` a beta branch; [person] promote the build to the default branch in App Admin.

## Open questions to settle before writing clients

1. Solana: pin to `@solana-mobile/dapp-store-publishing-tools` (Node) versus re-implement the tRPC procedures in Rust; ask Solana Mobile whether the portal's tRPC surface is a supported API and what its rate limits are. Decide wallet-delegated versus server-held signing.
2. Huawei: confirm `fileType` for `.app`, the release-state value table, and whether AGC exposes a certificate/profile API. Re-read `agcapi-upload-file-new` and `upload-url` when the site is up. Confirm that the Command Line Tools licence allows cloud use.
3. Apple (Mac App Store and hosted Linux uploads): confirm `buildUploads` accepts `MAC_OS` and `.pkg`.
4. Fees for Huawei, Samsung, Amazon, Meta and Snap, and review times for Samsung and Amazon: find official pages.
5. Microsoft: confirm what MSIX signing the Store accepts for upload from a non-Windows build host.
