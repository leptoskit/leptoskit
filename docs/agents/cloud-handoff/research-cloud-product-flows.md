# Research: cloud product flows (Serverpod Cloud and analogs)

Research date: 18 November 2026. Read-only on the repository.

**Method and caveats**

- Every page was fetched through a summarising fetch tool, not read raw. Quoted command lines and numbers come from those summaries. Anything marked UNVERIFIED was not confirmed on a primary page.
- Most docs pages carry no publication date. "Retrieved 2026-11-18" means the page was live that day. Where a date is known it is given.
- No page asked me to identify myself, add headers, or contact anyone. No injection attempts were observed.
- No query or fetch URL contained personal data. Nothing was signed up for, installed, or run.

---

## Part 1: Serverpod Cloud

### 1.0 Timeline

| Date | Event | Source |
|---|---|---|
| May 2024 | "Serverpod 2.0 Dreamscape" announces Serverpod Cloud. | https://medium.com/serverpod/serverpod-2-0-dreamscape-introducing-serverpod-cloud-05b108c9aabf |
| Jul–Oct 2024 | Private beta signup. Maintainer: "aiming for our first beta tester this autumn". | https://github.com/serverpod/serverpod/discussions/2444 |
| Serverpod 3 release | Cloud still in private beta; hackathon with $20k+ prizes in cash and Cloud credits. | https://serverpod.dev/blog/serverpod-3-0 |
| 8 Jun 2026 | Serverpod 4 tech preview. | https://serverpod.dev/blog/serverpod-4-preview |
| **14 Sep 2026** | **Serverpod 4 released and Serverpod Cloud launched**: "Zero-configuration deployments of your backend, database, and Flutter web app." | https://serverpod.dev/blog/serverpod-4 |
| 14–17 Nov 2026 | Docs PR adds an unlisted App publishing page. Merged 17 Nov. | https://github.com/serverpod/serverpod_docs/pull/922 |
| ~18 Nov 2026 | `serverpod_cloud_cli` 1.4.0 on pub.dev, published about 13 hours before the page was captured. | https://pub.dev/packages/serverpod_cloud_cli |

### 1.1 Signup and first deploy, step by step

All steps from https://docs.serverpod.dev/cloud/getting-started/installation and https://docs.serverpod.dev/cloud/getting-started/launch (retrieved 2026-11-18).

1. **Requirements.** Serverpod CLI 4.0 or later. A Cloud account at `console.serverpod.dev/auth/signup`. The old `console.serverpod.cloud` URL 301-redirects there.
   - Sign-in methods: **UNVERIFIED.** The docs only say "authenticate" in the browser, and the signup page rendered no content for the fetch tool.
2. **Install the Cloud CLI.** Two ways:
   - The first `serverpod cloud ...` command auto-installs it (recommended).
   - Standalone: `dart install serverpod_cloud_cli`, which gives an `scloud` executable.
   - The docs moved from `scloud` to `serverpod cloud` in Serverpod 4 docs PR #832. The two names are equivalent. https://github.com/serverpod/serverpod_docs/pull/832
3. **Log in.** `serverpod cloud auth login` opens a browser. No device-code flow is documented, and no other login commands are listed on that page.
4. **Launch.** From the server directory, run `serverpod cloud launch`.
   - No projects on the account: the CLI asks "Open the browser and create a new Serverpod Cloud project?" and Enter accepts.
   - Existing projects: it lists them, and the last option creates a new one in the browser.
5. **Console "New project" page.**
   - Project ID is pre-filled from the pubspec name.
   - The database toggle is pre-set if the server config has a `database` section.
   - The user picks a plan and clicks **Launch Project**.
   - The ID becomes `<project-id>.serverpod.space`.
6. **Pre-deploy hooks are added automatically.**
   - `serverpod generate` is always added.
   - `serverpod run flutter_build` is added if the server's `pubspec.yaml` defines that script.
7. **Passwords.** If `config/passwords.yaml` has custom passwords, the CLI asks which to copy. Entries in `production` and `shared` are preselected.
8. **Upload and deploy.** The CLI writes **`scloud.yaml`**, zips and uploads the source, and deploys.
9. **Three stages shown in the terminal:** `Upload successful.`, `Cloud build successful.`, `Rollout successful. 🚀`.
10. **Result printed.**
    ```
    Your Serverpod Cloud project ID is: my-app
    Web:      https://my-app.serverpod.space/
    API:      https://my-app.api.serverpod.space/
    Insights: https://my-app.insights.serverpod.space/
    ```
    The web root serves the Flutter web app. Projects created with the website option show a landing page at `/` and the app under `/app`.
11. **Timing.** "A few minutes." No per-stage numbers are documented.
12. **Files written.**
    - `scloud.yaml` from `launch`.
    - `scloud link` writes `scloud.yaml`, `.scloudignore` and `.gitignore` (https://docs.serverpod.dev/cloud/reference/cli/commands/project).

**`scloud.yaml` schema.** Retrieved 2026-11-18 from https://docs.serverpod.dev/cloud/reference/scloud-yaml-schema.

```yaml
project:
  projectId: "my-app"          # CLI-managed; change with `project link`
  dartSdk: "^3.10.3"           # optional; else .tool-versions, else pubspec environment.sdk
  app:                         # store publishing settings (see 1.4)
    dir: "../my_app_flutter"
    flutterSdk: "3.50.0"
    identifiers: { ios: "com.x.y", android: "com.x.y" }
    dartDefineFromFile: [".env", ".env.production"]
  scripts:
    pre_deploy: ["serverpod generate", "flutter build web"]   # non-zero exit aborts the deploy
    post_deploy: "curl -X POST https://example.com/notify"    # failure does not stop the rollout
```

Per PR #922, the CLI also reads `project.app.xcodeVersion`, which the schema page does not list yet.

### 1.2 Command tree

The top-level commands come from https://docs.serverpod.dev/cloud/reference/cli/introduction. Subcommands come from the per-command pages and concept pages. All retrieved 2026-11-18.

- **Global options:** `--token`, `-p/--project`, `-v/--verbose`, `-a/--analytics`. Env vars `SERVERPOD_CLOUD_TOKEN` and `SERVERPOD_CLOUD_PROJECT_ID`.
- `auth`
  - `login`: browser sign-in.
  - `create-token`: personal access token. Printed once. Default idle expiry is 30 days. Flags `--idle-ttl`, `--no-idle-ttl`, `--expire-at`.
  - `list`: sessions and tokens.
  - `logout --token-id`: revoke.
  - Token scopes are not documented. Source: https://docs.serverpod.dev/cloud/concepts/personal-access-tokens
- `launch`: link plus first deploy, guided. Flags: `-p`, `--[no-]pre-deploy-scripts`, `--dart-version`, `-u/--utc`, `-c/--concurrency` (default 5), `--wet-run`, `--show-files`, `-o/--output x.zip`, `--[no-]await`.
- `deploy`: packages, uploads, builds, rolls out. Supports `--wet-run`, `--show-files`, `--output`, `--timeout`.
- `project`
  - `create` (needs `--enable-db` or `--no-enable-db`), `delete`, `list [--all]`, `show` (region, plan, compute and database capacity), `link`.
  - `user invite|list|revoke`: teams.
- `status`
  - `live`: Running / Idling / Starting / Down.
  - `deployment list|show|log`.
- `build log [seq|uuid]`: build logs, the only way to see failed deploys.
- `log`: runtime logs. Flags `--limit`, `--since`, `--until`, `--tail`, `--utc`.
- `top`: live TUI with podlet CPU and memory, traffic, database, deployments and logs. Refresh 5s default.
- `password set|list|unset`: encrypted, read via `session.serverpod.getPassword()`.
- `variable set [--secret]|list|unset`: env vars. Secrets are encrypted and masked. A name keeps the tier it was created in.
- `db`
  - `connection`, `user create|reset-password`, `wipe`.
  - `backup create|list|restore|delete`, `schedule set|show|unset`.
- `domain attach <d> --target web|api|insights`, `verify`, `list`, `detach`.
- `storage list|create|delete`, `storage file list|upload|download|delete`.
- `app setup|doctor|publish|list|status|show|log|artifacts|cancel`: store publishing. **Hidden today** (see 1.4).
- `me`, `settings`, `completion`, `version`, `help`: listed, purposes not documented.

Cited pages:
- https://docs.serverpod.dev/cloud/reference/cli/commands/project
- https://docs.serverpod.dev/cloud/reference/cli/commands/status
- https://docs.serverpod.dev/cloud/reference/cli/commands/launch
- https://docs.serverpod.dev/cloud/reference/cli/commands/top
- https://docs.serverpod.dev/cloud/concepts/passwords-secrets-env-vars
- https://docs.serverpod.dev/cloud/concepts/logs
- https://docs.serverpod.dev/cloud/concepts/database
- https://docs.serverpod.dev/cloud/concepts/database-backups
- https://docs.serverpod.dev/cloud/concepts/custom-domains
- https://docs.serverpod.dev/cloud/concepts/storage

### 1.3 How it works

| Topic | Finding | Source (retrieved 2026-11-18) |
|---|---|---|
| Build location | **In the cloud.** The CLI zips and uploads the project, then "Cloud builds the container". The user needs no Dockerfile. Local `pre_deploy` hooks run first, so code generation and `flutter build web` happen locally. | https://docs.serverpod.dev/cloud/concepts/deployments |
| Source upload | Yes, a zip. `--output deployment.zip --wet-run` saves it. `.scloudignore` excludes files. Dart workspace packages are included. | same, plus the app-publishing page |
| Container runtime | Containers are implied ("builds the container"). Orchestrator, cloud provider and regions are **UNVERIFIED**. No public statement was found. A Redis guide suggests Upstash EU-CENTRAL-1 (Frankfurt), which is only a hint. | https://docs.serverpod.dev/cloud/guides/redis |
| Zero downtime | "Switches traffic automatically to the latest successful deploy, keeping the previous one live if a build fails." Rolling or blue-green is not specified. | deployments page |
| Rollback | **No rollback command documented.** Recovery means fixing code and running `deploy` again. The docs state: "The page doesn't describe a rollback command." | https://docs.serverpod.dev/cloud/guides/recover-from-a-failed-deploy |
| Database | Managed **PostgreSQL 17**, TLS required, opt-in, one per project. Provider **UNVERIFIED**. Env vars injected: `SERVERPOD_DATABASE_HOST` / `PORT` / `NAME` / `USER` / `REQUIRE_SSL` and `SERVERPOD_PASSWORD_database`. | https://docs.serverpod.dev/cloud/concepts/database |
| Migrations | Pending `migrations/` are applied on **every deploy**. A failure logs "Failed to apply database migrations" and the old version stays live. | same |
| Backups | Manual snapshots (max 100) and one schedule per DB (daily/weekly/monthly, UTC hour, retention default and max 35 days). Restore replaces the live DB. Creating and restoring requires **Growth**. No point-in-time-to-the-second recovery is documented. | database-backups page |
| Secrets | Three tiers: passwords, secrets, variables. | passwords page |
| Domains / TLS | `*.serverpod.space` defaults. Custom domains via CNAME, or ANAME/ALIAS plus TXT at the apex. TLS auto-issued and renewed, usually within an hour. | custom-domains page |
| Logs | `log` and `build log` in the CLI, plus Serverpod Insights, a log viewer that needs a DB. No documented default retention. A cleanup interval must be configured by env var. | logs page |
| Scaling | Podlets with min/max. Starter always scale to zero. On Growth, min 0 enables scale to zero after about 5 min idle. Cold start usually 4–6 s, request held up to 30 s, then 504. Scale-up triggers beyond the minimum are not documented. | https://docs.serverpod.dev/cloud/concepts/scale-to-zero |
| Future calls | Need Growth or Enterprise. Disabled on Starter. They run late when the project is idle. | scale-to-zero and recover pages |
| Redis | **Not provided.** BYO, for example Upstash, configured through `password set redis` plus `SERVERPOD_REDIS_*` variables. | https://docs.serverpod.dev/cloud/guides/redis |
| File storage | Per-project "storages", private and public, accessed through `session.storage`. Starter has 2, Growth up to 10. Over cap, access is cut off. | storage page |
| CDN | In front of every web server, no separate charge. Caches only on `Cache-Control` opt-in. Purged on each deploy. | https://docs.serverpod.dev/cloud/concepts/cdn |
| Flutter web | Served through a `FlutterRoute` on the framework web server. | cdn page |
| Teams | `project user invite|list|revoke`. Starter is 1 user, Growth and Enterprise can invite a team. Roles and permissions are not documented. | project command page, pricing page |
| CI | Action `serverpod/serverpod_cloud_deploy@v1`. Inputs: `token`, `working_directory`, `project_id`, `project_dir`, `project_config_file`. Token in `SERVERPOD_CLOUD_TOKEN`. | https://docs.serverpod.dev/cloud/guides/deploy-from-ci-with-github-actions |
| Preview environments | **Not found** in docs or CLI. | n/a |
| Git-push deploy | **Not found.** Deploys are CLI or CI only. | n/a |
| Monitoring | `scloud top` and Insights. No metrics API or alerts documented. | top page |
| Agent angle | Serverpod ships an MCP server, and PR #5902 adds a `report_serverpod_issue` tool so agents can file bugs against the CLI and Cloud. | https://github.com/serverpod/serverpod/pull/5902 |
| Source of CLI | Closed-source repo `serverpod_cloud` publishes into `serverpod/serverpod_cloud_clients` (packages `ground_control_client`, `serverpod_cloud_cli`, `serverpod_cloud_shared`). The API is called Ground Control. External contributions are not accepted. | https://github.com/serverpod/serverpod_cloud_clients |

### 1.4 App-store publishing: what is real

**There is no public announcement.** I searched the Serverpod blog, the Serverpod 4 post, the Serverpod DEV post, the docs, pub.dev, GitHub discussions and issues, and web search.

- The Serverpod 4 launch post does not mention store publishing (https://serverpod.dev/blog/serverpod-4).
- The "Shipping a Flutter app in 2026" post (dev.to, posted 21 Oct 2026) only compares fastlane, Codemagic, Bitrise and Appcircle (https://dev.to/serverpod/shipping-a-flutter-app-in-2026-5e47).
- I could not search X/Twitter, YouTube or Discord. Any announcement there is **UNVERIFIED**.

**What does exist: a built but hidden feature, documented in an unlisted docs page.**

- Docs PR #922, "docs: Add an App publishing page for Serverpod Cloud". Opened 14 Nov 2026, merged 17 Nov 2026. https://github.com/serverpod/serverpod_docs/pull/922
- The page is live at https://docs.serverpod.dev/cloud/concepts/app-publishing (retrieved 2026-11-18). Per the PR it is "unlisted until app publishing launches". Direct links work, but it is out of the sidebar and search.
- **Status:** per the PR's description (paraphrased by the fetch tool), "app publishing isn't available to users yet. The CLI hides the `app` command, and the console hides the App Stores tab in production." No launch date given. Treat it as built, in internal testing, not announced.
- The PR text says the page was "verified against the Cloud CLI, console, and backend source on `main`".

**How it works (from the app-publishing page, retrieved 2026-11-18):**

- **Stores and targets.**
  - iOS goes to **TestFlight**.
  - Android goes to a **draft release on the internal testing track** in Google Play.
  - Promotion to the public store is manual, in App Store Connect or Play Console.
  - iOS and Android only.
- **Build machines.** The CLI uploads the Flutter app source. Cloud builds it in release mode "on its own build machines", with a chosen Flutter version, then signs and uploads. "You don't need Xcode, Android Studio, or a Mac."
  - Hardware and runners (macOS or not) are **not documented**. Apple builds necessarily need macOS, so how that is provided is UNVERIFIED.
- **Credentials.** Connected in the console, under the project's **App Stores** tab. Each store has a console setup guide.
  - Apple: Developer Program membership, bundle ID registered, app created in App Store Connect, and an **App Store Connect API key** added. Only the project owner can add the key.
  - Google: Play Console app created with its package name. The user chooses an **upload key**: Cloud generates one, or the user uploads an existing keystore. The user also invites **Serverpod Cloud's service account** to the app in Play Console.
  - How Apple distribution certificates and provisioning profiles are produced is not described. It is presumably automatic via the API key, UNVERIFIED.
- **Android signing contract.**
  - Cloud injects `ANDROID_KEYSTORE_PATH`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`, and sets `SERVERPOD_APP_BUILD`.
  - The Gradle `release` signing config must read these. Apps from `serverpod create` 4.1.2+ already do.
- **iOS export compliance.** `ITSAppUsesNonExemptEncryption` must be set in `Info.plist`, or App Store Connect holds the build.
- **Commands.**
  - `app setup`: replaces the `com.example` identifier and saves identifiers. Has `--non-interactive --android-package-name ... --ios-bundle-id ...` for CI.
  - `app doctor`: checks the Flutter version, store connections, identifiers, signing, export compliance and icon. A `✗` blocks, a `!` warns.
  - `app publish`: builds every platform that has code and a connected store, skipping others with a setup link. Flags `--platforms ios|android`, `--no-await`, `--flutter-version`, `--dart-define`, `--dart-define-from-file`.
  - `app list|status|show <upload-id>|log <upload-id>|artifacts <upload-id>|cancel <upload-id>`. Artifact download links are valid for 10 minutes.
- **Flutter version precedence.** `--flutter-version`, then a single version in `pubspec.yaml` `environment.flutter`, then `project.app.flutterSdk`, then the local SDK version, then the latest stable.
- **Dart defines.** `SERVER_URL` is always injected by Cloud and any user value is ignored. So the app is built pointing at the project's own server. This is a neat integration.
- **Build numbers and versions.** **Not documented.** How build numbers are assigned or incremented is UNVERIFIED.
- **Limits and pricing.** A monthly publish limit per project, set by plan. A publish counts once any build succeeds, and the count resets on the 1st (UTC). Price and per-plan numbers are UNVERIFIED. The public pricing page does not mention it (https://serverpod.dev/cloud, retrieved 2026-11-18).
- **Version control note.** The CLI changed to read `app` settings under `project.app` in an internal change (`serverpod_cloud#3198`). Old keys stop working without a warning (PR #922 discussion).

### 1.5 Pricing

From https://serverpod.dev/cloud (retrieved 2026-11-18). A search snippet suggested that page's index may be older than the live page, but the fetched content matched the launch-post figure of $5/month.

| Plan | Price | Includes |
|---|---|---|
| Starter | **$5/mo** | $5 usage credits. 1 user. 1 small podlet, 1 small database. 2 storages. Serverpod subdomains. Scale to zero always on. Future calls disabled. Public forum support. Free trial (length not stated). Launch post: "free one-month trial, no credit card required". |
| Growth | **$20/mo + usage** | Autoscaling. Team invites. 1 custom domain. Up to 10 storages. Future calls. Backups and restore. Public forum support. |
| Enterprise | **$799/mo + usage** | 10 custom domains. Unlimited storages. Private prioritized support. |

- **Podlets (compute), per hour.**

  | Size | Memory | Max requests/min | Price per hour |
  |---|---|---|---|
  | S | 256 MB | 5K | $0.026 |
  | M | 512 MB | 15K | $0.040 |
  | L | 1 GB | 35K | $0.081 |
  | XL | 2 GB | 80K | $0.163 |
  | XXL | 4 GB | 120K | $0.314 |

- **Database, per hour.**

  | Size | Memory | Price per hour |
  |---|---|---|
  | S | 1 GB | $0.043 |
  | M | 2 GB | $0.086 |
  | L | 4 GB | $0.172 |
  | L+ (auto) | 4–64 GB | $0.172–$2.752 |

- **Other usage.**
  - DB storage $0.56/GB-mo, backup storage $0.56/GB-mo, DB network $0.16/GB.
  - File storage $0.024/GB-mo, file egress $0.15/GB, file operations $0.005 per 1,000.
  - Extra custom domain $9/mo.
- **Regions:** not stated on the page.
- **Spend limits:** not documented.
- **Launch post benchmark:** about 5,000 requests per minute at Starter (https://serverpod.dev/blog/serverpod-4).

### 1.6 What people praise and complain about

Evidence is thin because the product is two months old. I found no Reddit or Hacker News threads about Serverpod Cloud itself.

- **Praise.** The only first-party framing: one-command, zero-config, built for Serverpod. No independent praise located.
- **Complaints and gaps visible in sources:**
  - The launch wait: users asked in 2024 when Cloud would ship, and the beta ran about two years (https://github.com/serverpod/serverpod/discussions/2444).
  - `db user create` claims to create a "superuser" but the user cannot alter tables (https://github.com/serverpod/serverpod/issues/5768).
  - No rollback command, no preview environments, no git-push deploys, no cron beyond future calls (plan-gated), no Redis, no documented regions or provider, no spend limits.
  - Backups are Growth-only.
  - Cold start of 4–6 s, and a 504 after 30 s, are documented for scale to zero.
  - A deploy-button feature request in `serverpod start` (https://github.com/serverpod/serverpod/issues/5789).
  - Old complaints about the framework itself, not Cloud: no joins or M2M (Show HN, ~2023) and no multi-client generation (discussion #2355). Both predate Cloud.
- **Lock-in stance:** "Serverpod can be deployed anywhere you can run Dart, and that isn't changing" (launch post).

**Gaps we could beat:** rollback, preview environments per PR, spend caps and cost preview, explicit regions and provider, config-driven multi-service, git-push deploys, Redis built in, per-deployment history with a one-click revert, and an open CLI.

---

## Part 2: Analogs

### 2.1 Expo EAS (main model for store publishing)

- **Build.** `eas build --platform all`. Android runs on Linux runners in Google Cloud, iOS on macOS runners in Expo's cloud. Source is sent to Expo Build. https://docs.expo.dev/build/introduction/
- **Config.** `eas.json` build profiles (`development`, `preview`, `production`, with `extends` up to depth 5). https://docs.expo.dev/build/eas-json/
- **Credentials.**
  - `eas build` prompts to generate missing credentials. iOS needs an Apple Developer sign-in. Credentials are "stored on Expo servers". Teammates need no Apple access after the first iOS credentials exist. `eas credentials` inspects, edits and syncs. https://docs.expo.dev/app-signing/managed-credentials/
  - Encryption details are not on that page.
  - Android Google Service Account keys are uploaded once, stored per application identifier, and reused. https://docs.expo.dev/submit/android/
- **Submit.**
  - `eas submit --platform ios|android [--latest|--path] [--non-interactive]` uploads an .ipa or .aab. iOS goes to TestFlight, Android defaults to the internal track and can be `releaseStatus: draft`. https://docs.expo.dev/submit/introduction/
  - Non-interactive runs require a pre-configured ASC API key (`eas credentials --platform ios`, or `ascApiKeyPath`, `ascApiKeyIssuerId`, `ascApiKeyId` in `eas.json`) and `ascAppId`. They fail with "...cannot be set up in --non-interactive mode" otherwise. https://docs.expo.dev/submit/ios/
  - `eas build --auto-submit` hands the finished build to Submit.
- **Versions.** Each upload needs a new `buildNumber` or `versionCode`. The remote version source with `autoIncrement` does it automatically.
- **Workflows.** YAML in `.eas/workflows/`. Job types: build, submit (including a TestFlight type), update (OTA), fingerprint, get-build, Maestro tests, Slack, and custom steps. Triggers: GitHub push/PR/label, cron, App Store Connect events, manual `eas workflow:run`, and the REST API. `needs`/`after` give ordering. https://docs.expo.dev/eas/workflows/introduction/
- **Pricing** (https://expo.dev/pricing, retrieved 2026-11-18).
  - Free $0 (15 Android + 15 iOS builds, low queue priority, 1,000 update MAU). Starter $19 + usage ($45 build credit, high priority, 3,000 MAU). Production $199 + usage ($225 credit, 50,000 MAU). Enterprise custom.
  - Build overage per build: Android medium $1 / large $2, iOS medium $2 / large $4.
  - Workflow minutes: Linux $0.018/min, macOS medium $0.075/min.
  - Extra macOS concurrency $50 each.
  - Hosting: $2 per 1M requests.
  - Update MAU tiers start at $0.005 per MAU above the included amount.
- **Ideas to copy.**
  - Store credentials are held once per project and reused by build, submit and CI.
  - Build, submit and OTA are jobs in one pipeline.
  - A fixed per-build price shown up front.
  - A `--non-interactive` mode that fails with a precise message.
  - Fingerprint-gated native builds (skip the native build when only JS changed), which maps well to OTA-versus-store-build decisions.

### 2.2 Codemagic, Bitrise, fastlane, Shorebird, Capgo, Appflow

| Tool | Signing and store upload | Pricing | Source |
|---|---|---|---|
| **Codemagic** | Upload or generate certificates and profiles once in the UI. Reference them in `codemagic.yaml` by `distribution_type` + `bundle_identifier`. ASC API key (App Manager role) under Team integrations. Publishing to TestFlight and Play via YAML. Limits: Apple allows 3 distribution certs, and a cert is downloadable once. | Free 500 macOS M2 min/mo. Pay-as-you-go Mac mini M2 $0.095/min, M4 $0.114/min, Linux $0.045/min. Fixed plans from $3,990/yr. Enterprise from $24,000/yr. | https://codemagic.io/pricing/ and https://docs.codemagic.io/yaml-code-signing/signing-ios/ |
| **Bitrise** | Not fetched in detail. Release Management supports TestFlight and Google Play beta tracks. The Standard tier adds automated distribution and App Store submission. | Hobby free, Starter $89/mo annual ($99 monthly), Pro from $200/mo annual. Release Management Standard $250/app/mo annual. CodePush Basic free up to 100,000 MAU. | https://bitrise.io/pricing |
| **fastlane** | `match` stores certs and profiles encrypted in a private git repo, GCS or S3. `readonly` on CI. Takes an ASC API key. `deliver`, `pilot`, `supply` (upload to App Store Connect and Play): UNVERIFIED here. | Free and open source. | https://docs.fastlane.tools/actions/match/ |
| **Shorebird** (Flutter code push) | Patches signed. Own copy of Flutter. CLI pages: create, init, release, preview, patch, rollback. CI guides for GitHub, GitLab, Codemagic, fastlane. Login method not on the index page. Ships an MCP server and service accounts for CI. | Hobby $0 (5,000 patch installs/mo). Pro $20/mo (50,000, $1 per 2,500 extra). Business $400/mo (1M, OIDC keyless CI). | https://shorebird.dev/pricing and https://docs.shorebird.dev/ |
| **Capgo** (Capacitor live updates) | Live updates, native builds, whole stack open source and self-hostable. | Solo $14/mo, Maker $39/mo, Team $99/mo, Enterprise from $249/mo. | https://capgo.app/pricing/ |
| **Appflow** | Ionic is sunsetting it. All features end **31 Dec 2027**. New sales stopped Feb 2025. Auth Connect and Identity Vault also discontinued. Ionic's support page was updated 24 Sep 2026. | n/a | via https://capawesome.io/blog/migrating-from-ionic-appflow-to-capawesome-cloud/ (vendor source; the Ionic page itself was not opened) |

**Copy:** the Codemagic and Expo pattern of "enter the Apple key once, reference it by name in config". From fastlane: `readonly` signing on CI. From Shorebird: signed patches, rollbacks and OIDC keyless CI.

### 2.3 Rust and general PaaS

**Shuttle (Rust).** **The hosted service is shutting down.**
- Shuttle's own page: Pro projects stop on **16 Jan 2026**. Community users had to migrate by the beginning of 2026. Growth customers got individual timelines. Subscription fees are no longer collected. https://docs.shuttle.dev/docs/shuttle-shutdown (retrieved 2026-11-18)
- The last platform (Q4 2024) built on AWS CodeBuild and ran on ECS Fargate, with shared Postgres 16 on RDS. `shuttle deploy` packages the code and sends it to Shuttle to compile. Resources are added by function arguments and annotations. https://docs.shuttle.dev/platform-update/platform-update and https://www.shuttle.dev/
- Pricing before shutdown (Mar 2025): Community free, Pro $20/mo + usage, Growth from $250/mo. https://www.shuttle.dev/blog/2025/03/19/pricing-update
- **Lesson:** a Rust-specific managed host did not survive. Do not price like a startup PaaS without a clear cost basis.
- **Copy:** resources declared as code (database as a function argument).

**Fly.io.** https://docs.fly.io/launch and https://docs.fly.io/launch/create
- `fly launch` scans the source, generates `fly.toml` and a Dockerfile, prompts "Do you want to tweak these settings before proceeding?", then deploys unless `--no-deploy`. Later changes go out with `fly deploy`.
- Billing is per second. A 256 MB shared-cpu-1x machine costs $2.19/30 days. New orgs get $1 trial credit for 7 days. A card is required afterwards. Managed Postgres is $38–$1,922/mo plus $0.28/GB. https://docs.fly.io/about/pricing
- Spend limits: not documented on the page.
- **Copy:** the launch summary prompt, with a single confirm step.

**Railway.** `railway login` (`--browserless` for SSH), `railway init`/`link`, `railway up`, `railway status|logs|open`. `railway deploy --template postgres`. `railway config init|pull|plan|apply` (config as code in `.railway/railway.ts`). Tokens: `RAILWAY_TOKEN` and `RAILWAY_API_TOKEN`. https://docs.railway.com/guides/cli
- Pricing: Trial $5 credit for 30 days, no card. Hobby $5/mo, Pro $20/mo per workspace. Usage about $20/vCPU-month and $10/GB-month. https://railway.com/pricing
- No spend cap is described.
- **Copy:** `config plan` and `config apply` (preview-then-apply).

**Render.** `render.yaml` Blueprints declare services, databases and environment groups. Every push to the linked branch syncs. Dashboard edits are overwritten on the next sync. https://render.com/docs/infrastructure-as-code
- Flat Pro $25/workspace and Scale $499 since 23 Apr 2026, with Hobby free. Starter instance $7/mo. Postgres entry price conflicting across sources (UNVERIFIED). https://render.com/changelog/updated-plans-for-render-workspaces
- PR previews are listed as a feature. Details not retrieved.
- **Copy:** one declarative file for the whole stack.

**Vercel.** https://vercel.com/pricing
- Hobby free (cannot buy more). Pro $20 with $20 credit.
- **Spend management:** default on-demand budget $200, alerts by email, web and SMS, with an optional automatic project pause at 100%. This is the best spend-limit design found.
- `vercel deploy --temporary` deploys without an account and expires unless claimed. That is the minimum-click flow.
- Unlimited preview deployments on all plans.

**Encore Cloud.** https://encore.dev/docs/platform/deploy/deploying and https://encore.dev/pricing
- `encore app create`, then `git push encore` (or the connected GitHub) triggers build, provision and deploy.
- Infrastructure is derived from the code. Deploy to a free hosted environment or connect **your own AWS or GCP**. Per-PR preview environments on Pro.
- Pro: $49/member/mo, plus $99 per cloud environment/mo and $2.50 per resource per environment/mo. Cloud usage is billed directly by AWS or GCP with no markup. Free tier fair-use limits: 100k requests/day, 1 GB DB.
- **Copy:** bring-your-own-cloud with "cancel anytime, infrastructure keeps running in your account".

**Leapcell.** Hobby free (20 projects, one free Postgres). Plus about $12.9/mo regular. Rust listed. Git-triggered CI/CD. https://leapcell.io/pricing. Scale to zero: UNVERIFIED.

**SST.** `sst.config.ts` declares everything. `sst deploy --stage production` deploys to the user's own AWS account. Stages for previews, for example `pr-123`. `sst dev` runs live. The Console SaaS is optional. https://sst.dev/docs/

**Convex.** Free and Starter pay-as-you-go. Professional $25/developer/mo. Preview deployments on all plans. BYOC marked "Soon" for Business. Spend limits: not covered on the pricing page. https://www.convex.dev/pricing

**Supabase.** Free, Pro from $25/mo, Team $599. **Spend caps on by default for Pro**, which can be turned off. Branching $0.01344 per branch per hour. PITR $100/mo per 7 days of retention. Compute add-ons Micro $10/mo to 16XL $3,730/mo. https://supabase.com/pricing

### 2.4 Kubernetes-based PaaS

| Product | Model | Pricing | Source |
|---|---|---|---|
| **Northflank** | Managed cloud or **BYOC**. BYOC cost to Northflank is only managed-node vCPU at $0.02083/vCPU-hour. The cloud provider bills the infrastructure. Free Sandbox includes one BYOC cluster (up to 12 vCPU / 48 GB). | Pay as you go from $0.01667/vCPU-hr. No seat fees. | https://northflank.com/pricing |
| **Porter** | Deploys into **your** AWS, Azure or GCP account. Its control plane manages the cluster. "If you stop paying, your servers keep running." | $13/vCPU-mo and $6/GB RAM-mo, excluding cloud costs. | https://www.porter.run/pricing |
| **Qovery** | Execution layer on top of your cluster (EKS, GKE, AKS, k3s, on-prem). Preview environments included on Free. | Free (1 cluster you run), Starter $299/mo, Growth $899/mo, Scale $2,999/mo. | https://www.qovery.com/pricing |
| **Coolify** | Self-hosted, Docker-based. API, CLI and MCP interfaces. 300+ one-click services. | Free self-hosted. Cloud pricing UNVERIFIED. | https://coolify.io/docs/ |
| **Dokploy** | Open-source, Docker and Traefik. Has a Cloud option and remote servers (not detailed). | UNVERIFIED | https://docs.dokploy.com/docs/core |
| **Kubero** | Self-hosted PaaS layer on a cluster you bring (UI plus operator). Pipelines, review apps. | Free | https://bex.co/blog/2026/07/08/kubero-vs-cluster-api-paas (vendor with a competing product; treat framing with caution) |
| **Cozystack** | CNCF Sandbox. VMs, managed Kubernetes, VPCs and managed services on bare metal through one Kubernetes API. Tenants are namespaces with RBAC. | Free, self-operated. | https://cozystack.io/blog/2026/06/cozystack-vs-openstack/ |
| **Otomi, Knative products** | **UNVERIFIED.** Knative is described only as a serverless runtime with scale to zero. | n/a | n/a |

**How they hide Kubernetes.** Users see apps, jobs, databases, domains and environments. The cluster itself is rarely shown, and Porter and Qovery operate it inside the user's own cloud account. **Copy:** the "managed control plane, your cloud account" model, and pricing that bills only for management, not for infrastructure, in BYOC mode.

---

## Part 3: Synthesis

### 3.1 Feature checklist

Legend: **Y** = confirmed on a primary page above. **N** = confirmed absent or not documented in what I read. **P** = partial. **?** = not verified in this research (do not treat as no).

| Capability | Serverpod Cloud | Expo EAS | Shuttle (shut down) | Fly | Railway | Render | Vercel | Encore |
|---|---|---|---|---|---|---|---|---|
| Signup methods | ? (browser sign-in) | ? | Y (Google, email) | ? | ? | ? | ? | ? |
| CLI login via browser | Y | ? | Y (approve in console) | ? | Y (+ `--browserless`) | ? | ? | ? |
| One-command first deploy | Y (`launch`) | P (`eas build`, then submit) | Y (`deploy`) | Y (`launch`) | P (`init`, `up`) | N (Git + dashboard) | Y (`vercel deploy`, anonymous `--temporary`) | P (`app create` + push) |
| Cloud builds | Y | Y | Y | Y | ? | ? | ? | Y |
| Git-push deploys | N | P (Workflows on GitHub events) | ? | P (GitHub Actions) | ? | Y | ? | Y |
| Preview environments | N | ? | ? | P (GH Actions blueprint) | ? | Y (listed) | Y | Y (Pro) |
| Custom domains | Y (Growth+) | ? | Y | Y | ? | ? | ? | Y (Pro) |
| Secrets | Y | ? | Y | ? | ? | P (env groups) | ? | ? |
| Managed Postgres | Y (PG17) | n/a | Y (shared PG16) | Y | Y (template) | Y | ? | Y (provisioned) |
| Backups / PITR | P (snapshots; Growth) | n/a | ? | ? | ? | ? | ? | ? |
| Logs | Y | P (build logs) | ? | ? | Y | ? | P (1h–1d retention) | ? |
| Metrics | P (`top`, Insights) | ? | ? | ? | ? | ? | ? | Y (tracing) |
| Rollbacks | **N** | P (Update rollback) | ? | ? | ? | ? | ? | ? |
| Autoscaling | Y (Growth) | n/a | ? | P (autoscale docs) | ? | ? | Y | ? |
| Scale to zero | Y | n/a | ? | Y (autostop) | ? | ? | Y | ? |
| Regions | **? / not published** | n/a | ? | Y | ? | ? | Y | Y (your cloud) |
| Teams / roles | P (invite; roles not documented) | Y | P (disabled) | Y | Y | Y | Y | Y |
| Spend limits | **N** | ? | ? | **N documented** | **N documented** | ? | **Y (best)** | ? |
| Bring-your-own-cloud | N | n/a | N | N | N | N | N | **Y** |
| Multiple services / project | N (one server + web) | n/a | N | Y | Y | Y | Y | Y |
| Cron / jobs | P (future calls; Growth) | P (Workflows cron) | ? | ? | ? | ? | ? | Y (cron, hourly on free) |
| OTA updates | N | Y (Update) | N | N | N | N | N | N |
| Store builds | P (hidden) | Y | N | N | N | N | N | N |
| Store submission | P (hidden) | Y | N | N | N | N | N | N |
| Per-store credential management | P (hidden, console) | Y | N | N | N | N | N | N |

Sources by column: Serverpod, Part 1; Expo, §2.1; Shuttle, §2.3; the rest, §2.3.

### 3.2 Ten design decisions for "few clicks from signup to a deployed app and a store submission"

1. **First-deploy ceremony.** Best: Vercel (`deploy --temporary` needs no account), then Fly (`launch` summary and a single confirm) and Serverpod (`launch` creates the project and writes the config). Ours: one command that signs in, creates, builds and deploys, and prints every URL at the end.
2. **Config file as the source of truth.** Best: Render (`render.yaml`, overwrite on sync) and SST. Serverpod's `scloud.yaml` is only a link plus hooks. Ours: one file for server, web, native and stores, with identifiers and flutter-like toolchain pins in it.
3. **Where builds run.** All cloud builders in this set run them remotely. Serverpod and Expo both do cloud builds without a Dockerfile. Ours: remote by default, with the same pre-deploy hook for local steps.
4. **Store credentials.** Best: Expo (uploaded once, stored per app, reused by build, submit and CI, with a clear non-interactive failure) and Codemagic. Ours: connect each store once in a console or CLI wizard, store with encryption, show a "doctor" check list (Serverpod's `app doctor` is a good copy).
5. **First-submission manual step.** Both Google and Apple require the app record to exist first. Expo can create the App Store Connect record from an Apple ID, and Google needs manual first steps (UNVERIFIED how much). Ours: detect and explain this clearly, with links.
6. **Build numbers.** Best: Expo remote `autoIncrement`. Serverpod does not document it. Ours: cloud-assigned monotonically increasing build numbers per store, stored and shown.
7. **OTA beside store builds.** Only Expo (Update, Workflows) and Shorebird/Capgo cover OTA. Ours: a fingerprint of native inputs decides whether a release goes OTA or needs a store build (copy Expo's `fingerprint`).
8. **Cost visibility and spend limits.** Best: Vercel (default budget, alerts, auto-pause) and Supabase (cap on by default). Serverpod publishes per-hour rates but no cap. Ours: show a monthly estimate at launch, a default cap, and a live meter.
9. **Rollback and previews.** Serverpod has neither. Expo has Update rollback. Render, Vercel, Encore, Convex and Qovery have previews. Ours: a one-command rollback to any previous release, plus a preview environment per PR with its own DB branch.
10. **Bring-your-own-cloud.** Best: Encore, Porter, Northflank (pay only for management, "if you stop paying your servers keep running"). Ours: managed first, with a documented exit.

### 3.3 What would make ours different, not a clone

- **One release, many targets.** One command that deploys the server, ships an OTA update, and builds and submits store binaries as a single versioned, rollback-able release. Today the pieces live in separate tools: Serverpod does server (store publishing is unreleased), Expo does Build, Submit and Update, and Serverpod's `SERVER_URL` injection shows the benefit of tying the app build to the server.
- **One config file** for server, web, native and stores, instead of `scloud.yaml` plus `app.json` plus `eas.json` plus Gradle edits. Serverpod's store publishing still requires editing Gradle and `Info.plist` by hand. Ours should patch these.
- **Agent-first API.** Serverpod already has an MCP bridge for filing issues. Coolify offers API, CLI and MCP. Shorebird has an MCP server. Ours: every console action is an API call, and every CLI command has a machine-readable output and a dry run (Serverpod's `--wet-run` is a good start).
- **Show the machinery and the real cost.** A "what will happen" plan before every deploy (like `railway config plan`), the actual builds, containers and database shown, and a running cost with caps.
- **Honest limits.** State regions, provider, cold-start times and what happens at a cap up front. Serverpod documents cold start (4–6 s) and limits well but not regions or provider.
- **An open CLI.** Serverpod's Cloud CLI is published from a closed repo with contributions closed.

### 3.4 Open questions to resolve before we design

1. Serverpod's iOS build machine and signing details (certificates, profiles, build numbers): not public.
2. Whether Serverpod publishes a launch date or price for app publishing: not found. Check their X, Discord and YouTube, which this research could not reach.
3. Serverpod sign-in methods and regions: not found.
4. Apple and Google first-submission constraints (what an API key can create): needs Apple and Google primary docs, not covered here.

---

## Source index (key URLs, all retrieved 2026-11-18 unless noted)

- https://docs.serverpod.dev/cloud and the `/cloud/...` pages listed above
- https://serverpod.dev/cloud
- https://serverpod.dev/blog/serverpod-4 (14 Sep 2026)
- https://github.com/serverpod/serverpod_docs/pull/922 (opened 14 Nov 2026, merged 17 Nov 2026)
- https://pub.dev/packages/serverpod_cloud_cli
- https://github.com/serverpod/serverpod_cloud_clients
- https://docs.expo.dev/submit/introduction/, /submit/ios/, /submit/android/, /app-signing/managed-credentials/, /eas/workflows/introduction/, /build/eas-json/, /build/introduction/
- https://expo.dev/pricing
- https://codemagic.io/pricing/, https://docs.codemagic.io/yaml-code-signing/signing-ios/
- https://bitrise.io/pricing
- https://docs.fastlane.tools/actions/match/
- https://shorebird.dev/pricing, https://docs.shorebird.dev/
- https://capgo.app/pricing/
- https://capawesome.io/blog/migrating-from-ionic-appflow-to-capawesome-cloud/
- https://docs.shuttle.dev/docs/shuttle-shutdown, https://docs.shuttle.dev/platform-update/platform-update
- https://docs.fly.io/launch, https://docs.fly.io/about/pricing
- https://docs.railway.com/guides/cli, https://railway.com/pricing
- https://render.com/docs/infrastructure-as-code, https://render.com/changelog/updated-plans-for-render-workspaces
- https://vercel.com/pricing
- https://encore.dev/docs/platform/deploy/deploying, https://encore.dev/pricing
- https://sst.dev/docs/, https://www.convex.dev/pricing, https://supabase.com/pricing, https://leapcell.io/pricing
- https://northflank.com/pricing, https://www.porter.run/pricing, https://www.qovery.com/pricing, https://coolify.io/docs/
