# Cloud design direction

Design direction for the LeptosKit Cloud console: the leptoskit app in `crates/leptoskit_cloud_app` where a developer signs up, ships an app, and runs it. It replaces the hosted pilot's console (`crates/leptoskit_app/src/screen.rs`) in spirit. The product, its vocabulary and its API come from the cloud architecture contract (the planning note the foundation slice adds as `docs/agents/cloud.md`); section numbers like "contract §6" refer to it. The design language is the admin's ([admin-design.md](admin-design.md)) and the brand's ([`docs/src/brand.md`](../src/brand.md)).

Everything here is buildable from leptoskit components compiled to Wasm and run as the same tree in the iOS and Android cloud apps. There is no hand-written JavaScript. Where the design needs something leptoskit can't do yet, §19 lists it as a gap with the feature it asks for; nothing below works around a gap silently.

## 1. Concept: you sign the sheet, the cloud stamps it

Every hosting console has a deploy button, a log, a cost page and an audit trail, each on its own screen. Ours folds them into one object people can hold in their head: **the release sheet**.

A release sheet is a sheet of paper that says what will ship where, what it costs, and what it can't do, with a line at the bottom for your signature. You sign it to ship. Then each place it ships to **stamps** it as the release lands: `BUILT`, `MIGRATED`, `LIVE`, `IN REVIEW`. A failed release is stamped `FAILED` on the line that failed, with the reason and the fix written beside it. A release headed for the Solana dApp Store also needs a **seal**: your publisher wallet signs it, because the store asks for that and we never hold your wallet.

The sheet is the deploy button, the cost estimate, the timeline, and the audit log at once:

- **Before you sign**, it's the plan: one line per target (server, each service, migrations, web assets, the over-the-air update for each platform, each store), with the monthly cost and the honest fine print ("A rollback won't undo migration 0007").
- **While it ships**, it's the timeline: the stage rail across the top moves through `queued → building → migrating → rolling out → live`, and each line gets its stamp as its target lands.
- **Afterwards**, it's the record: who signed, from where (the web, a terminal, a CI token), and every stamp with its time. Rolling back is signing a new sheet whose source is an old one.

This is the contract's "one release, many targets" made visible. Nobody else draws a server deploy, an over-the-air update and three store submissions as one thing with one signature; here it's the only thing you ever look at.

### Why a signature

- **It's the one human act in a release, so we draw it as one.** Pressing "Deploy" feels like nothing; signing feels like you meant it. The signature is your name, written in the brand's hand face in pen-blue ink, with where it came from: "Signed off by Ana Lima, from the terminal on ana-mbp". A release from CI is signed by its token ("Signed off by token `ci-deploy`, GitHub Actions"), so the record is honest about who, or what, decided.
- **It's the same act as the wallet seal.** Most releases are signed off with your name. A dApp Store release is also sealed with your key. Teaching "sign here" on every ordinary release makes the one cryptographic signature in the product feel natural instead of alarming.
- **It's how people already think about shipping.** "Who signed off on this?" is a question every team asks after a bad release. The sheet answers it without a separate audit page.

The first screen of the product teaches it: the sign-up card is a sheet, and the place you sign up is its signature line (§7).

### Workings: every card has a back

The admin's signature, **Workings**, carries over as the family resemblance: every card, line and stat has the same small mint tab (the logo's mint piece tilted −12° and the word `Workings`), and `W` and `Shift+W` do what they do in the admin. What changes is the room behind the door. In the admin the tab opens a drawer beside the card. In the cloud **the card turns over**: its back is the ink underside, in the same place and at the same size, showing the machinery that card is made of.

| Card (front)       | Its back                                                                                      |
| ------------------ | --------------------------------------------------------------------------------------------- |
| Now serving        | The Deployment, its ReplicaSet and pods: names, images by digest, ages, restarts              |
| A topology node    | That process's Deployment, Service, HTTPScaledObject (sleep), NetworkPolicy and ResourceQuota |
| This month's cost  | The arithmetic: size × hours × the catalogue price, with the price's source and date          |
| A domain           | The custom hostname, its certificate, the route object, and the last DNS answer we saw        |
| The database       | The CloudNativePG cluster or shared database and role, the backup schedule object, WAL target |
| A release sheet    | Five stations (§9.6): Declare, Build, Spec, Apply, Cost                                       |
| A store connection | The credential names in the vault, their versions and who set them, and the last check's call |

Why turn the card over instead of opening a drawer:

1. **It works on a phone.** A drawer beside a card has nowhere to go at 400px; the admin falls back to a bottom sheet, which leptoskit can't present accessibly yet (no focus scope, §19). A card that turns over in place needs no overlay, no focus trap, and no second layout.
2. **The front and back are the same thing.** The pods are on the back of "Now serving" because they _are_ what's serving. Spatial memory does the teaching: people learn that the cost card's back is arithmetic and the domain card's back is DNS without reading a label.
3. **It stays the family's idea.** Mint still means "this opens how it works", the underside is still ink in both themes, the stations still read top to bottom on a mint rail. Someone who uses the admin and the cloud learns one gesture.

A back is an index card, not a document: it's the size of the front and shows the three to six things that matter, with **All workings →** to the full view (§9.6). The release sheet is the exception: it's a page, so its back is a page.

**Turn everything over.** `Shift+W`, or **Show workings** in the account menu, turns every card on the screen over in a 40ms stagger, and the overview becomes the underside of your app: Deployments, Services, scalers, certificates and prices in the places their cards were. It's the X-ray the admin's Workings mode is, as one gesture. Press it again to turn everything back.

### Alternatives considered and rejected

1. **The topology as the whole console** ("the map is the menu": you navigate by pressing the drawing of your app). It's visual, but Railway's project canvas already owns it, it's a pan-and-zoom surface on a phone, and leptoskit has no pan gesture on the web or in the generated iOS and Android apps yet. The drawing stays as one card on the overview (§10), and pressing its nodes navigates; it isn't the navigation.
2. **Budget-first scaling** (you set what you'll pay; the cloud picks the size and replicas). Appealing for hobbyists, but price-to-shape is many-to-one ($10 buys an always-on `s` or a sleeping `m` that autoscales), so the cloud would have to choose behaviour for you, which is the opposite of showing your workings. What survives: every size option shows its price under it, the price total updates as you press, and the spend cap is set in money (§11).
3. **A split-flap departures board** for releases and targets. Familiar from status pages, low density, and its per-character animation wouldn't run in native apps (motion is web-only today). The sheet's stamps carry the same "landed / delayed / cancelled" information with more room for the reason.
4. **The leptoskit mark assembling as deploy progress** (build is the first ember box, migrate the second, rollout lands the mint piece). On-brand, but the admin reserves the assembling mark as the product's one celebration (finishing the Learn tracks); spending it on every deploy cheapens both. The cloud's one celebration is the first `LIVE` stamp on your first release.

### Where the cloud sits between the public site and the admin

The public site and docs are Skribble ink: sketch lettering for headings and body, rough frames, hatching. The admin is the calm product register of the same palette: system sans, hairlines, ember for data, mint for machinery. The cloud is a daily tool, so it sits **with the admin**, with three deliberate departures towards the site:

1. **The first-run screens borrow the site's voice.** Sign-up, `/welcome`, new project, and the first deploy are where people arrive from the public site. Their headings use the sketch face at display size, so the step from the landing page to the console doesn't feel like a different company. From the first live release on, headings are system sans.
2. **The release sheet is paper.** It's the one surface drawn as an object: warmer sheet colour, ruled lines, an ember margin rule, the hard offset block under it, a signature in the hand face, and stamps. Everything around it is calm.
3. **The hand face means a person.** The admin uses the sketch face only for the admin talking to you (margin notes). The cloud keeps that and adds signatures, which are a person's mark. Numbers, prices, and machine output are never handwritten.

Status, prices and logs never borrow the site's playfulness. The fun is in the sheet, the stamps and the turn-over, not in the chrome.

## 2. Pushback on the brief

- **No command palette.** ⌘K is in every console, so it doesn't distinguish ours, and it needs an overlay with a focus trap that leptoskit can't present accessibly yet (§19). The cloud has few screens per project, every screen is a URL, the rail lists every project, and the CLI is the power user's palette: every action shows its `lk cloud …` command on its back. `/` focuses the rail's project filter; that covers "go to".
- **Five sign-in methods of equal weight is a wall of logos.** Developers deploying an app mostly have GitHub, and GitHub is also what makes "Import from GitHub" one press later. So: **Continue with GitHub** first, **Email me a code** second, and passkey, Google and Solana wallet as a quiet row. A passkey becomes the first choice for returning people who added one. The wallet button appears only where a wallet is installed (it can't work otherwise), and the wallet's real job, sealing dApp Store releases, is in Stores, where it matters (§13.6).
- **"Import from GitHub" is the biggest of the three new-project paths and has no operations in the contract** (no GitHub App installation, no repository list, no way to read a repository's `leptoskit.toml` before deploying). It's designed below so the page has a place for it, but it should ship after the starter and terminal paths, which the contract already supports (§20 lists the operations it needs).
- **A ticking "live" cost meter is alarming and dishonest.** Usage is sampled (contract §7, `sample_usage`), not streamed, so a counter ticking every second would invent precision we don't have. The meter shows "so far this month" and a forecast, says when it was last updated ("3 min ago"), and changes when a sample arrives. It's never red unless the forecast crosses your cap.
- **"Usage by meter" as a page of charts is generic.** Usage is shown as a **receipt**: one line per meter with quantity, unit price and amount, the included credit, and the total. One chart survives: spend so far against the cap, with the forecast dashed (§14.4).
- **Organisations are invisible to one person.** Sign-up creates a personal organisation silently. The organisation switcher, roles and members appear when there's a second organisation or a second member, not before.
- **Scale changes in the console fight "one file".** Contract §5 puts size, replicas and sleep in `leptoskit.toml`. If the console changes them, the next `lk deploy` from the file changes them back. The design doesn't hide that: a console change is shown as **differs from `leptoskit.toml`** with the TOML to paste, and the next release sheet has a line for any scale it will reset (§11). Whether the console should be allowed to change scale at all is open question 1 (§22).
- **Request metrics aren't in the API.** The contract has logs and usage but no operation for request rate, latency or errors, so the overview's health card shows what the runtime reports (ready replicas, restarts, sleeps, wake times, out-of-memory kills) and leaves request charts to the app's own admin. Adding a metrics operation is listed in §20, not designed around.
- **Operator views stay plain.** They're for a handful of people; they get tables and the same components, and no stamps beyond status words.

## 3. Information architecture

### Routes

The cloud runs at `https://cloud.leptoskit.dev` by default (contract §5's `url`; a self-hosted cloud has its own). Every screen is a typed route in the file router, so the web and both native apps share the paths, and a push notification or a link from the CLI opens the exact screen. Organisation and project slugs are path segments, like GitHub's.

| Route                                                          | Screen                                                                                      | Section |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ------- |
| `/sign-in`, `/sign-in?next=<path>`                             | Sign in or sign up (one screen)                                                             | §7.1    |
| `/sign-in/email?to=<address>`                                  | Enter the emailed code                                                                      | §7.2    |
| `/device`, `/device?user_code=WDJB-MJHT`                       | Approve a terminal (`lk cloud login`)                                                       | §7.4    |
| `/welcome`                                                     | The first thirty seconds, once                                                              | §7.3    |
| `/invitations/<token>`                                         | Accept an invitation                                                                        | §14.2   |
| `/todo`                                                        | Everything waiting on you, across organisations                                             | §13.5   |
| `/account`                                                     | Your sign-in methods, passkeys, wallets, sessions, appearance                               | §14.5   |
| `/organizations`                                               | Your organisations; create one                                                              | §14.1   |
| `/<org>`                                                       | Projects                                                                                    | §10.1   |
| `/<org>/new`, `?path=terminal\|starter\|github`                | New project                                                                                 | §8      |
| `/<org>/new/github`                                            | Import: pick a repository                                                                   | §8.3    |
| `/<org>/<project>`, `?env=<env>`                               | Project overview                                                                            | §10.2   |
| `/<org>/<project>/releases`, `?env=<env>&failed=true`          | Release history                                                                             | §9.8    |
| `/<org>/<project>/<env>/<n>`, `?log=events\|build\|runtime`    | Release sheet `n` in an environment                                                         | §9      |
| `/<org>/<project>/<env>/ship`, `?from=<n>` or `?again=<n>`     | A draft sheet: roll back to `n`, or ship `n`'s source again                                 | §9.5    |
| `/<org>/<project>/<env>/logs`, `?process=&since=&q=`           | Runtime logs                                                                                | §9.7    |
| `/<org>/<project>/<env>/scale`                                 | Scale and cost                                                                              | §11     |
| `/<org>/<project>/<env>/variables`                             | Variables and secrets                                                                       | §12.1   |
| `/<org>/<project>/<env>/domains`, `…/domains/<hostname>`       | Domains, and one domain's DNS                                                               | §12.2   |
| `/<org>/<project>/<env>/database`                              | Database and backups                                                                        | §12.3   |
| `/<org>/<project>/stores`, `…/stores/<store>`                  | Stores; connect and check one                                                               | §13     |
| `/<org>/<project>/publish`                                     | A draft sheet for store releases                                                            | §13.3   |
| `/<org>/<project>/sign/<request>`                              | Seal a release with your wallet                                                             | §13.6   |
| `/<org>/<project>/settings`                                    | Name, environments, delete                                                                  | §12.4   |
| `/<org>/settings/members`, `/tokens`, `/billing`               | Organisation                                                                                | §14     |
| `/operator`, `/operator/clusters/<id>`, `/workers`, `/pairing` | Operator (operator role only)                                                               | §15     |
| any route + `?workings=<card>`                                 | The same screen with that card's full workings open (the right column, or a page on phones) | §9.6    |

Turning a card over is local state and isn't in the URL; it's a glance. **All workings →** is a link (`?workings=<card>`), so "look at this" can be sent to a teammate.

The paths add reserved words the contract's slug rules don't have yet (§20): organisation slugs can't be `sign-in`, `device`, `welcome`, `invitations`, `todo`, `account`, `organizations` or `operator`; project slugs can't be `new` or `settings`; environment names can't be `releases`, `stores`, `publish`, `sign`, `settings`, `ship` or `logs`.

### Navigation

**Desktop (≥ 1024px).** A top bar and a left rail, the admin's shell:

```text
┌───────────────────────────────────────────────────────────────────────────────────────────┐
│ ◰  acme / notes  [production]        ● notes v42 · Building 1:12    $4.12 of $20    To do 2   AL │  top bar, h-14
├──────────────┬────────────────────────────────────────────────────────────────────────────┤
│ acme         │                                                                            │
│ ⌕ Filter  /  │   content, max-w-[1200px], 12 columns                                      │
│ PROJECTS     │                                                                            │
│ ▸ blog       │                                                                            │
│ ▾ notes      │                                                                            │
│   Overview   │                                                                            │
│   Releases   │                                                                            │
│   Logs       │                                                                            │
│   Scale      │                                                                            │
│   Variables  │                                                                            │
│   Domains    │                                                                            │
│   Database   │                                                                            │
│   Stores     │                                                                            │
│   Settings   │                                                                            │
│ + New project│                                                                            │
│ ORGANISATION │                                                                            │
│   Members    │                                                                            │
│   Tokens     │                                                                            │
│   Billing    │                                                                            │
└──────────────┴────────────────────────────────────────────────────────────────────────────┘
```

- **Top bar**: the mark (to `/<org>`), the breadcrumb (`acme` to the projects, `notes` to the overview; each a link), the environment pill on project pages (to the overview's environment row), then, right-aligned: the **shipping chip** (any release in flight in this organisation, newest first; "3 shipping" when there are several; press for the sheet or a list), the **cost chip** ("$4.12 of $20", to billing; warn-coloured with `!` when the forecast crosses the cap), **To do** with a count, and the avatar (to `/account`).
- **Rail** (`w-60`): the organisation, a filter field, the projects (the current one expanded to its pages), **New project**, and the organisation's pages. Members, Tokens and Billing show only to roles that can open them. **Operator** appears for the operator role.
- With two or more organisations, the rail's organisation name gains **Switch** (to `/organizations`). With one, there's nothing to switch, so nothing shows.

**Phone (< 640px).** No rail. A compact top bar and a bottom tab bar:

```text
┌──────────────────────────────┐
│ ◰  notes        [production] │  top bar, h-12; "‹ notes" on sub-pages
├──────────────────────────────┤
│                              │
│  content, one column, px-4   │
│                              │
├──────────────────────────────┤
│  Apps   Releases  To do²  You │  tab bar, h-16 + pb-safe
└──────────────────────────────┘
```

- **Apps** is the projects list. **Releases** is every release in the organisation, newest first, so the phone is where you watch things ship. **To do** is the inbox: wallet seals, store steps only you can take, invitations, a reached cap. **You** is the account, organisations, members, tokens and billing.
- A project's pages are the rows of an **In this project** list at the bottom of its overview (Releases, Logs, Scale, Variables and secrets, Domains, Database, Stores, Settings), the way phone settings screens work. Sub-pages show "‹ notes" in the top bar.

**Tablet (640–1023px).** The rail collapses to icons (`w-16`, labels in `accessibility_label`), content uses 6 columns.

### Switching organisation, project and environment

- **Organisation**: `/organizations` lists them; the rail's **Switch** goes there. The last organisation you opened is where `/` sends you.
- **Project**: the rail on desktop, **Apps** on phones. No dropdown: leptoskit has no popover with focus handling yet, and a list you can see beats one you have to open.
- **Environment**: the overview's environment row, a wrapping row of pills (`production` first, then `staging`, then previews by recency). Environment-scoped pages (logs, scale, variables, domains, database) carry the environment in the path, so switching keeps you on the same page: from `/acme/notes/production/logs`, pressing `staging` goes to `/acme/notes/staging/logs`. On sub-pages the top bar's pill links back to the row.

### Keys

Desktop web and the macOS window, handled by one `on_key` on the root view: `/` focuses the project filter; `W` turns the focused card over; `Shift+W` turns every card over; `Esc` turns cards back and closes the right column. That's all; no chords to learn.

### Layout grid

| Width                   | Rail              | Content                         | Right column (logs, full workings)                                                         |
| ----------------------- | ----------------- | ------------------------------- | ------------------------------------------------------------------------------------------ |
| ≥ 1280px (`xl`)         | `w-60`            | 12 columns, `gap-4`, max 1200px | `w-[440px]` beside the content; while open the rail collapses to `w-16` and content uses 6 |
| 1024–1279px (`lg`)      | `w-60`            | 12 columns                      | a section after the content, full width, with a "Back to the sheet" link                   |
| 640–1023px (`sm`, `md`) | `w-16` icons      | 6 columns                       | a section after the content                                                                |
| < 640px                 | none; the tab bar | 1 column, `px-4`                | a section after the content, or its own page from `?workings=`                             |

The right column is never an overlay: below 1280px it becomes part of the page. That's simpler to build, needs no focus trap, and reads the same to a screen reader as it looks.

The shell is the admin's: `h-screen flex-col`, the top bar, a `flex-row flex-1` of rail, content (`ScrollView`, `flex-1`) and the optional column, then the tab bar on phones. The compiler has no `fixed` or `sticky`; pinned chrome comes from the layout.

## 4. Tokens

The cloud installs its tokens with `Theme::set_color` at mount, as the admin does, under a `--cloud-` prefix, and keeps class constants in `leptoskit_cloud_app::style` so no screen spells a colour. Values reuse the admin's wherever the role is the same; additions and changes are marked **new** or **changed**. Contrast is WCAG 2.2, computed for the surface the token is used on.

### Colour

| Token                      | Light     | Dark      | Use                                                                 | Contrast                                 |
| -------------------------- | --------- | --------- | ------------------------------------------------------------------- | ---------------------------------------- |
| `--cloud-paper`            | `#faf6f0` | `#16111f` | page background                                                     |                                          |
| `--cloud-surface`          | `#fffdf8` | `#1d1726` | cards, rail, top bar, tab bar                                       |                                          |
| `--cloud-sunken`           | `#f3ede4` | `#251e30` | inputs, wells, skeletons, log background                            |                                          |
| `--cloud-line`             | `#e8e0d4` | `#30283c` | hairlines, card borders (decorative)                                |                                          |
| `--cloud-line-strong`      | `#cdbfad` | `#4a3f58` | dividers you need to see (decorative)                               |                                          |
| `--cloud-control-line` new | `#8a7f96` | `#827591` | borders of inputs, steppers, toggles, size boxes                    | ≥ 3.25:1 light, ≥ 3.75:1 dark (non-text) |
| `--cloud-ink`              | `#16111f` | `#f4efe8` | text, primary button fill (light)                                   | 17.2:1 / 16.2:1                          |
| `--cloud-ink-2`            | `#4d4558` | `#c9c0d0` | secondary text                                                      | 8.5:1 / 9.9:1                            |
| `--cloud-ink-3`            | `#6e6578` | `#9d93a8` | muted text, timestamps                                              | 5.1:1 (4.75:1 on sunken) / 6.0:1         |
| `--cloud-ember`            | `#e5304f` | `#ff5a73` | the mark, the focus ring, your app's accent in drawings; never text | 4.2:1 on surface (non-text)              |
| `--cloud-ember-text` new   | `#c42a47` | `#ff5a73` | ember words: "Serving", the staging pill, seal labels               | 5.5:1 / 5.8:1                            |
| `--cloud-ember-fill` new   | `#c42a47` | `#ff5a73` | the production pill, the seal                                       | with `--cloud-on-ember`: 5.6:1 / 6.1:1   |
| `--cloud-on-ember` new     | `#ffffff` | `#16111f` | text on `--cloud-ember-fill`                                        |                                          |
| `--cloud-ember-warm`       | `#ff7a45` | `#ff8a5c` | the primary button's offset block                                   |                                          |
| `--cloud-mint`             | `#14bf9b` | `#5ef2c2` | the mint tab's piece, the Workings rail                             |                                          |
| `--cloud-mint-text`        | `#0a6e59` | `#5ef2c2` | mint words on paper                                                 | 5.5:1 / 13.2:1                           |
| `--cloud-mint-wash`        | `#e3f6ee` | `#11302a` | a value that just changed, margin notes                             |                                          |
| `--cloud-underside`        | `#16111f` | `#0f0b16` | the back of every card (always ink)                                 | text `#f4efe8`: 16.2:1                   |
| `--cloud-underside-line`   | `#30283c` | `#2a2236` | rules on the back                                                   |                                          |
| `--cloud-button`           | `#16111f` | `#f4efe8` | primary button fill                                                 |                                          |
| `--cloud-button-text`      | `#faf6f0` | `#16111f` | primary button text                                                 | 17.2:1                                   |
| `--cloud-sheet` new        | `#fffaf0` | `#221b2c` | the release sheet's paper (the brand's Skribble paper in light)     | ink 17.8:1 / 14.6:1; ink-3 5.3:1 / 5.7:1 |
| `--cloud-rule` new         | `#efe4d4` | `#2c2437` | the sheet's ruled lines (decorative)                                |                                          |
| `--cloud-block` new        | `#16111f` | `#4a3f58` | the hard offset block under raised paper (ink would vanish in dark) |                                          |
| `--cloud-pen` new          | `#27348b` | `#a9b4ff` | signatures: pen-blue ink, the one place a person's hand appears     | 10.3:1 / 8.5:1 on the sheet              |

Status colours are the admin's and are reserved for state, always with an icon and a word:

| Token          | Light     | Dark      | Icon and word, and the stamps that use it                                          | On the sheet  |
| -------------- | --------- | --------- | ---------------------------------------------------------------------------------- | ------------- |
| `--cloud-good` | `#1f7a45` | `#5fd08a` | ✓ Live, Running, Active · stamps `BUILT` `MIGRATED` `ROLLED OUT` `LIVE` `RELEASED` | 5.1:1 / 8.6:1 |
| `--cloud-warn` | `#9a5b00` | `#e3b341` | ! Waiting, Needs you, Over cap soon · stamps `WAITING` `IN REVIEW` `UNKNOWN`       | 5.2:1 / 8.6:1 |
| `--cloud-bad`  | `#b8233f` | `#ff8a9e` | ✕ Failed, Crashing, Cap reached · stamp `FAILED`                                   | 6.0:1 / 7.4:1 |
| `--cloud-info` | `#4d4558` | `#c9c0d0` | • Queued, Asleep, Cancelled · stamps `QUEUED` `SKIPPED` `CANCELLED` `SUPERSEDED`   | 8.8:1 / 9.5:1 |

`SEALED` uses `--cloud-ember-fill`: a seal is yours, not the machine's.

**Changes from the admin, and why.** The admin fills its production pill with `#e5304f` under white 12px text (4.3:1) and colours the staging pill's text the same ember (4.2:1); both are under 4.5:1. The cloud uses `--cloud-ember-fill` and `--cloud-ember-text` instead. The admin's input borders use `--admin-line-strong` (1.7:1), under the 3:1 that WCAG asks of a control's boundary; the cloud uses `--cloud-control-line`. The admin should take both fixes.

### Type

The admin's scale, plus four roles. Faces are `--cloud-font-sans` (the system UI face), `--cloud-font-mono` (Skribble Mono Playful, then `ui-monospace`) and `--cloud-font-hand` (Skribble Noto Sketch). Font families, `tabular-nums`, `truncate` and `tracking-*` apply on the web only today; native apps draw the system face (§19, gap 1).

| Role                                                    | Face | Size and weight | Classes                                                                               |
| ------------------------------------------------------- | ---- | --------------- | ------------------------------------------------------------------------------------- |
| First-run display (sign-up, new, first release) **new** | hand | 36px            | `font-[var(--cloud-font-hand)] text-[36px] leading-[1.15]` (28px under `sm`)          |
| Page title                                              | sans | 24px semibold   | `text-2xl font-semibold leading-tight`                                                |
| Release number on a sheet **new**                       | sans | 36px semibold   | `text-4xl font-semibold leading-none tabular-nums`                                    |
| Section and card title                                  | sans | 16px semibold   | `text-base font-semibold`                                                             |
| Body, table cells, sheet lines                          | sans | 14px            | `text-sm leading-normal`                                                              |
| Labels, timestamps                                      | sans | 12px medium     | `text-xs font-medium`                                                                 |
| Group label (capitals in the source)                    | sans | 12px semibold   | `text-xs font-semibold tracking-wider`                                                |
| Hero number (cost so far)                               | sans | 32px semibold   | `text-3xl font-semibold leading-none tabular-nums`                                    |
| Prices and quantities                                   | sans | as the row      | `tabular-nums`                                                                        |
| Code, commands, logs, hostnames                         | mono | 13px            | `font-[var(--cloud-font-mono)] text-[13px] leading-relaxed`                           |
| Stamp text **new**                                      | mono | 11px semibold   | `font-[var(--cloud-font-mono)] text-[11px] font-semibold tracking-wider leading-none` |
| Signature **new**                                       | hand | 30px            | `font-[var(--cloud-font-hand)] text-[30px] leading-none text-[var(--cloud-pen)]`      |
| Margin notes                                            | hand | 16px            | `font-[var(--cloud-font-hand)] text-[16px] leading-snug`                              |

Stamp words are written in capitals in the source string (there's no `uppercase` utility) and always carry an `accessibility_label` in sentence case, so a screen reader says "Live at 14:05", not letters.

### Space, radius, elevation, rules

- Spacing is Tailwind's 4px scale, as the admin: card padding `p-4 md:p-5`, `gap-4` between cards, `gap-8` between sections, `gap-3` inside a card. Sheet padding is `px-5 py-6 md:px-8 md:py-8`; sheet lines are `min-h-12` rows.
- Radius: cards `rounded-xl`, controls `rounded-lg`, pills `rounded-full`, **sheets `rounded-[6px]`** (paper has nearly square corners), **stamps `rounded-[4px]`**, seals `rounded-full`.
- Elevation, without shadows: page, then card (`--cloud-surface` + `border border-[var(--cloud-line)]`), then **raised**: a sibling `View` behind the element, `absolute inset-0 translate-x-1 translate-y-1` (sheets `translate-x-1.5 translate-y-1.5`), filled `--cloud-block`, or `--cloud-ember-warm` under the primary button. Pressed raised buttons move onto their block (`active:translate-x-1 active:translate-y-1`).
- **Rules are 1px `View`s, never per-edge borders.** UIKit, AppKit and Win32 don't paint per-edge borders yet (CSS coverage), so a `border-l-[3px]` margin line would vanish on iOS. The sheet's ruled lines, its ember margin rule and the margin note's pencil line are each a `View` with `h-px` or `w-px` (or `w-[3px]`) and a background.
- Focus: a 2px `--cloud-ember` ring, drawn as `border-2` on a wrapper whose `focused` signal the element's `Focus` and `Blur` events set; never removed.

### Motion

`entering` presences and Tailwind transitions run on the web, iOS, macOS and Android; `Motion` (the pen-stroke clip reveals) runs on the web only and shows elements at rest elsewhere. Every row has a reduced-motion form; §19 gap 2 is the switch that makes transitions honour it.

| Name      | What moves                         | Web                                                                                                                                  | iOS, macOS, Android                                             | Reduced motion                 |
| --------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------- | ------------------------------ |
| Press     | a raised button onto its block     | `transition-transform duration-75 active:translate-x-1 active:translate-y-1`                                                         | same classes                                                    | kept: 4px of direct feedback   |
| Turn over | a card to its back and back again  | front `transition-transform duration-150 ease-in scale-x-0`; back `transition-transform scale-x-100 delay-150 duration-150 ease-out` | same classes                                                    | faces swap at once             |
| Turn all  | every card on the screen           | as "Turn over", each card delayed by 40ms × its index                                                                                | same                                                            | at once                        |
| Write     | a sheet line appearing             | `Motion::enter(Pose::new().clip(Inset::from_left(0.0)))`, 500ms `Easing::Out`, parent `stagger` 90ms                                 | `entering=Presence::fade().slide_by(-8.0, 0.0).duration(200.0)` | at rest                        |
| Sign      | the signature being written        | the same clip reveal over 900ms `Easing::InOut`                                                                                      | `Presence::fade().duration(300.0)`                              | at rest                        |
| Stamp     | a stamp landing                    | `entering=Presence::fade().scale(1.4).rotate(r - 6.0).spring(Spring::SNAPPY)`                                                        | same                                                            | at rest                        |
| Changed   | a number or line that just changed | `bg-[var(--cloud-mint-wash)]` then transparent, `transition-colors duration-1000`                                                    | same (backgrounds animate everywhere)                           | the wash shows for 1s, no fade |
| Running   | the current stage's dot            | `animate-pulse`                                                                                                                      | same                                                            | static, the word carries it    |
| Column    | the right column opening           | `entering=Presence::slide(Edge::Right).duration(200.0)`                                                                              | same                                                            | at once                        |

`r` is the stamp's resting rotation, picked from the release ID so it never jiggles between renders: one of `-rotate-6`, `-rotate-3`, `-rotate-2`, `rotate-2`, `rotate-3`.

### Class constants

Screens never spell a class list; they use these constants from `leptoskit_cloud_app::style`. The admin's constants (`style.rs`) carry over with `--admin-` renamed to `--cloud-`: `PAGE`, `CARD`, `CARD_HEADER`, `CARD_TITLE`, `PAGE_TITLE`, `BODY`, `SECONDARY`, `MUTED`, `LABEL`, `GROUP_LABEL`, `HERO_NUMBER`, `CODE`, `BUTTON_PRIMARY` and `BUTTON_PRIMARY_BLOCK`, `BUTTON_QUIET`, `MINT_TAB`, `MINT_PIECE`, `UNDERSIDE`, `CODE_BLOCK`, `CODE_BLOCK_PAPER`, `CHIP`. These change or are new:

| Constant            | Classes                                                                                                                                                                                         | Note                                                       |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| `SHELL`             | `h-screen flex-col bg-[var(--cloud-paper)] text-[var(--cloud-ink)]`                                                                                                                             | new                                                        |
| `TOP_BAR`           | `h-12 sm:h-14 flex-row items-center gap-3 px-4 bg-[var(--cloud-surface)]`, with a 1px rule `View` under it                                                                                      | new                                                        |
| `RAIL`              | `w-16 lg:w-60 flex-col gap-1 px-2 py-3 bg-[var(--cloud-surface)]`, with a 1px rule `View` beside it                                                                                             | new; hidden under `sm`                                     |
| `RAIL_ITEM`         | `min-h-11 flex-row items-center gap-3 rounded-lg px-3 text-sm text-[var(--cloud-ink-2)] hover:bg-[var(--cloud-sunken)]`                                                                         | new                                                        |
| `RAIL_ITEM_CURRENT` | `RAIL_ITEM` + `bg-[var(--cloud-sunken)] font-semibold text-[var(--cloud-ink)]`, and `aria_current`                                                                                              | new; the current page is weight and fill, not colour alone |
| `TAB_BAR`           | `h-16 flex-row items-stretch bg-[var(--cloud-surface)] pb-safe`                                                                                                                                 | new; phones only                                           |
| `TAB`               | `flex-1 flex-col items-center justify-center gap-0.5 text-[11px] font-medium text-[var(--cloud-ink-3)]`; current: `text-[var(--cloud-ink)] font-semibold` + a 2px ink bar `View` above the icon | new                                                        |
| `CONTENT`           | `flex-1 px-4 md:px-6 xl:px-8 py-6 gap-8 max-w-[1200px]`                                                                                                                                         | new                                                        |
| `BUTTON_SECONDARY`  | as the admin, with `border-[var(--cloud-control-line)]`                                                                                                                                         | changed: control contrast (§4)                             |
| `BUTTON_DANGER`     | as the admin; `BUTTON_DANGER_QUIET` is `BUTTON_QUIET` with `text-[var(--cloud-bad)]`                                                                                                            | quiet variant new                                          |
| `INPUT`             | `min-h-11 rounded-lg border border-[var(--cloud-control-line)] bg-[var(--cloud-sunken)] px-3 text-sm text-[var(--cloud-ink)]`                                                                   | changed: control contrast                                  |
| `INPUT_ERROR`       | `INPUT` with `border-2 border-[var(--cloud-bad)]`, and the message under it with ✕                                                                                                              | new                                                        |
| `PILL_PRODUCTION`   | `rounded-full px-2 py-0.5 text-xs font-semibold bg-[var(--cloud-ember-fill)] text-[var(--cloud-on-ember)]`                                                                                      | changed: contrast (§4)                                     |
| `PILL_STAGING`      | `rounded-full px-2 py-0.5 text-xs font-semibold border border-[var(--cloud-ember-text)] text-[var(--cloud-ember-text)]`                                                                         | changed                                                    |
| `PILL_PREVIEW`      | `rounded-full px-2 py-0.5 text-xs font-semibold bg-[var(--cloud-sunken)] text-[var(--cloud-ink-2)]`                                                                                             | new                                                        |
| `NOTE`              | `flex-row items-start gap-3 rounded-[10px] bg-[var(--cloud-mint-wash)] p-3`, with a `w-[3px]` mint `View` as its first child                                                                    | changed: no per-edge border (§4, rules)                    |
| `SKELETON`          | `rounded-lg bg-[var(--cloud-sunken)]`                                                                                                                                                           | new                                                        |
| `FOCUS_RING`        | `rounded-[10px] border-2 border-[var(--cloud-ember)]` on the wrapper while focused; `border-2 border-transparent` otherwise, so nothing shifts                                                  | new                                                        |
| `TARGET`            | `min-h-11 min-w-11 items-center justify-center`                                                                                                                                                 | new; every pressable that isn't already that size          |

The sheet's constants (`SHEET`, `STAMP`, `SIGNATURE` and the rest) are in §9.2, beside the screen that defines them.

## 5. Components

### Built-in leptoskit builders the screens use

`View`, `Text`, `Heading`, `Pressable`, `Button`, `TextInput`, `PasswordInput`, `Switch`, `ActivityIndicator`, `LinearProgressIndicator`, `ScrollView`, `Image`, `Vector` and `VectorPath`, `Grid`, `Wrap`, `Stack` and `Positioned`, `Divider`, `For`, `Show`, the toasts in `leptoskit::components::toast`, and `VirtualList` from `leptoskit_virtual` for logs. Clipboard writes go through `leptoskit_plugin_clipboard`; native sign-in through `leptoskit_plugin_web_auth` and `leptoskit_plugin_passkeys`; push through `leptoskit_notifications`; QR codes from `leptoskit_auth::client::qr`, which already draws them as `Vector` paths.

The generated iOS, macOS and Android apps draw `Switch`, `Checkbox`, `Radio`, `Slider`, `LinearProgressIndicator` and `ActivityIndicator` (`MobileWidgetControls.swift`, `NativeWidgetRenderer.kt`), although the component reference's "Backend availability" paragraph still says they don't; that paragraph needs updating. The cloud uses the real `Switch` for every on/off choice, `ActivityIndicator` for busy buttons and `LinearProgressIndicator` for quota bars. Its size boxes, choice rows and steppers are built from `Pressable` and `View` with the right `role` and `aria_*` props, because they show a price under each option, which no radio widget does. Every control is a tap target; none needs a gesture, which also suits that pan gestures don't reach the web or the generated apps yet.

### From `leptoskit_admin`

The admin's look carries over; its behaviour doesn't, because the admin wires drawers, confirms, copying and live updates in `crates/leptoskit_admin/assets/admin.mjs`, which the cloud can't use. Recommendation for UI 1: move the admin's look-only builders into a small shared crate (`leptoskit_design`, unpublished at first), with behaviour as signals in Rust, so the two products can't drift and the admin can later drop its script.

| Piece                                       | File in `crates/leptoskit_admin/src`                                                                 | Use in the cloud                                              |
| ------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| Tokens and class constants                  | `style.rs` (`tokens_css`, `CARD`, `BUTTON_*`, `MINT_TAB`, `UNDERSIDE`, `CODE_BLOCK`, `NOTE`, `CHIP`) | the base of `leptoskit_cloud_app::style`, renamed `--cloud-*` |
| `status_word`, `Tone`                       | `components.rs`                                                                                      | every state word                                              |
| `card`, `Card`                              | `components.rs`                                                                                      | the front of `TurnCard`                                       |
| `mint_tab`                                  | `components.rs`                                                                                      | the Workings door on every card                               |
| `raised`                                    | `components.rs`                                                                                      | primary buttons, the sheet                                    |
| `margin_note`                               | `components.rs`                                                                                      | first-run notes                                               |
| `env_pill`                                  | `components.rs`                                                                                      | with the colour fixes in §4                                   |
| `meter`, `bar`                              | `components.rs`                                                                                      | storage, quota, cost against cap                              |
| `code_block`, `paper_code`, `code`, `prose` | `components.rs`                                                                                      | commands, TOML, Kubernetes objects, logs' code                |
| `fact`, `tabs`, `empty_state`               | `components.rs`                                                                                      | "Now serving", log tabs, empty states                         |
| `line_chart`, `sparkline`, formatters       | `charts.rs`                                                                                          | spend against cap; `bytes`, `count`, `milliseconds`           |
| `Drawer`, `Station`, `Body`                 | `workings.rs`                                                                                        | the stations on a card's back and the full workings           |
| `mark`                                      | `shell.rs`                                                                                           | the top bar                                                   |

### New components

Each lists its props and every state. "Focused" is the wrapper ring from §4 on every pressable and isn't repeated.

| Component            | Props                                                                                                                                                      | States                                                                                                                                      |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `ReleaseSheet`       | number, environment, source (upload digest, commit and message, starter, rollback of `n`), lines, totals, fine print, signature, overall stamp, stage rail | draft (unsigned), signing, queued, building, migrating, rolling out, live, superseded, failed { stage, blocker }, cancelled, unknown (§9.3) |
| `SheetLine`          | target kind and icon, title, detail, where, cost, progress (done, total, label), stamp                                                                     | planned, in progress (with progress), stamped, failed (with the blocker's one-line reason), skipped (with why)                              |
| `StageRail`          | the five stages, each pending, current { elapsed }, done { duration }, skipped { why }, failed                                                             | per station as listed; the rail itself `role(List)`, the current station `aria_current`                                                     |
| `Stamp`              | kind, time, place, rotation, size (`line` 20px tall, `sheet` 44px tall)                                                                                    | entering (lands once), at rest, faded (`opacity-60` on superseded sheets)                                                                   |
| `Signature`          | signer, via (web, terminal and host, token and its client), time                                                                                           | unsigned (the **Sign and ship** button on a dotted line), signing (busy), signed                                                            |
| `Seal`               | wallet short address, item count                                                                                                                           | waiting, preparing, signing { n of m }, sealed, declined, expired, wrong wallet, low balance, sent and unconfirmed                          |
| `TurnCard`           | front, back, card ID, `turned` signal, full-workings link                                                                                                  | front, turning, back                                                                                                                        |
| `BlockerPanel`       | blocker, stage, detail, who can act                                                                                                                        | one per blocker (§9.4); actions disabled with a reason for roles that can't act                                                             |
| `Topology`           | processes, routes, database, devices (from the release's spec and the runtime's observation)                                                               | per node: running, asleep, waking, rolling out { from, to, ready of total }, crashing { restarts }, stopped at cap, unknown                 |
| `CostMeter`          | spent so far, forecast, cap, plan, updated at                                                                                                              | under cap, forecast over cap (warn), cap reached (bad), free plan (quota bars instead), stale (no sample for 30 min)                        |
| `Receipt`            | lines (label, quantity, unit price, amount), included credit, total or range, cap                                                                          | estimating, estimated, changed (mint wash on changed lines), stale (estimate older than the last change), error                             |
| `SizePicker`         | sizes with CPU, memory and hourly price, selected, plan                                                                                                    | per box: selected, available, locked by plan (with the plan that unlocks it)                                                                |
| `Stepper`            | label, value, minimum, maximum                                                                                                                             | default, at minimum, at maximum, disabled { reason }                                                                                        |
| `Toggle`             | label, on, description                                                                                                                                     | on, off, disabled { reason }; a labelled row around the real `Switch`, the whole row pressable                                              |
| `ChoiceRow`          | options, selected                                                                                                                                          | per option selected or not; `role(Role::RadioGroup)` with `Role::Radio` children, arrow keys move                                           |
| `ChangeSlip`         | pending changes, cost delta, effect sentence, TOML to paste                                                                                                | hidden (nothing pending), pending, applying, applied, failed                                                                                |
| `LogView`            | source (events, build, runtime), process filter, time range, query, follow                                                                                 | loading, following, paused { new lines }, empty, ended, older lines expired (retention), disconnected                                       |
| `CommandBlock`       | commands                                                                                                                                                   | idle, copied (for 2s, announced)                                                                                                            |
| `ListeningChecklist` | items                                                                                                                                                      | per item pending, active (pulsing dot), done, failed; the admin's listening empty state, generalised                                        |
| `DnsRecords`         | records (type, name, value), last check                                                                                                                    | per record not found, found but different (shows what we saw), matches                                                                      |
| `ConfirmPanel`       | sentence, what's lost, confirmation text, action                                                                                                           | ready, typed text doesn't match, busy, done, failed. Inline in the page, never an overlay                                                   |
| `StepList`           | steps from the store report (`publish::steps::Step`: store, by, what, url, fields, why, done when)                                                         | per step to do, done (detected), done (marked), not now; per list "n of m done"                                                             |
| `StoreTile`          | store, connection, last release and its stamp, doctor summary                                                                                              | not connected, connected, needs attention, checking                                                                                         |
| `DoctorList`         | checks                                                                                                                                                     | per check ready, warning, blocks, checking                                                                                                  |
| `OnceValue`          | value, what it is                                                                                                                                          | shown, copied, dismissed (gone for good)                                                                                                    |
| `ShippingChip`       | releases in flight                                                                                                                                         | hidden, one, several                                                                                                                        |
| `EnvRow`             | environments, selected                                                                                                                                     | per pill production (ember fill), staging (ember outline), preview (sunken fill), selected (ink underline under the pill)                   |
| `OfflineChip`        | last update time                                                                                                                                           | hidden, offline, back online (2s, announced)                                                                                                |
| `QrCode`             | URL                                                                                                                                                        | at rest; always paired with the URL as text and a copy button                                                                               |

## 6. Voice

The admin's voice (admin-design §8) holds: plain, second person, present tense; the number or outcome first; things named as they are in the code; buttons are verbs with objects; destructive confirms say exactly what's lost; no apologies, no exclamation marks; success is quiet. The cloud adds:

- **Releases ship, and people sign off.** "Sign and ship v42", "Signed off by Ana Lima", "Ship v41 again" (a rollback), "Stop building v42" (cancel). Never "deploy succeeded", "execute", or "trigger".
- **Stamps are one or two words**, the same everywhere: `QUEUED`, `BUILT`, `MIGRATED`, `ROLLED OUT`, `LIVE`, `UPLOADED`, `IN REVIEW`, `RELEASED`, `WAITING`, `SKIPPED`, `CANCELLED`, `SUPERSEDED`, `FAILED`, `UNKNOWN`, `SEALED`.
- **Money is ranges and arithmetic, never adjectives.** "$4.00 – $12.00 a month", not "from just $4". "+$4.00 a month" for a change, in ink, never red; only crossing your cap is a warning. Free things say what they are: "Free on Hobby; sleeps when idle".
- **Limits are stated where they bite.** "The first request after sleeping waits while it starts: 1.2s last time." "A rollback doesn't touch the database." "Play review takes hours to days; we can't speed it up."
- **When we don't know, we say so, and say what we're doing about it.** "We lost contact with the worker during rollout. We're checking what's actually running before we say more." Never guess a failure.
- **Who can act is named.** "Only owners and billing members can raise the cap: Ana Lima, Ben Ode."

## 7. Sign-up and sign-in

People come from three places: the public site's **Start** link, a link the CLI printed, and an invitation. All three land on one screen; there's no separate sign-up page, because the only difference is whether an account exists, and the server knows that, not the person.

The operations here are `leptoskit_auth`'s client functions (`providers`, `sign_in_with_provider`, `send_code`, `verify_code`, `passkey_sign_in`, `wallet_apps`, `wallet_sign_in`, `merge_offer`), then the cloud's `account::whoami` and `account::organizations`.

### 7.1 `/sign-in`

**Purpose**: get a developer in with one press, or back to the terminal that sent them.

The card is a sheet (§9 primitives: sheet paper, ruled lines, the offset block). The place you sign up is its signature line, so the product's one gesture is the first thing anyone does.

```text
Desktop: centred on --cloud-paper, max-w-md          Phone: the same sheet, full width less px-4

┌──────────────────────────────────────────────────┐
│ ◰ LeptosKit Cloud                                 │
│                                                   │
│ Sign here.                         ← hand, 36px    │
│ One account for your apps' servers, databases,    │
│ over-the-air updates and store releases.          │
│ ───────────────────────────────────────────────── │  ← --cloud-rule, 1px View
│ ✓ A server and database for one app, free         │
│ ✓ Every release can be rolled back                │
│ ✓ A spend cap from the first minute               │
│ ───────────────────────────────────────────────── │
│ [ Continue with GitHub                        ]▌  │  ← primary, raised, h-11 (h-12 on phones)
│                                                   │
│ [ you@example.com             ] [ Email me a code ]│  ← secondary; wraps under the field on phones
│                                                   │
│ Passkey  ·  Google  ·  Solana wallet               │  ← quiet buttons, only those that can work here
│ ───────────────────────────────────────────────── │
│ By continuing you accept the Terms and the        │  ← fine print, ink-3, text-xs
│ Privacy notice.                                   │
└──────────────────────────────────────────────────┘
```

- **Components**: the sheet classes (`SHEET`, `SHEET_BLOCK`, `SHEET_RULE` in §9.2), `raised(Button)`, `TextInput().autocomplete("email")`, quiet `Pressable` links with icons, `Text` for fine print.
- **Order and emphasis**: GitHub is the primary button. Email code is the full-width secondary path. Passkey, Google and Solana wallet are a quiet row: **Passkey** only when the platform has passkeys, **Solana wallet** only when `wallet_apps()` finds one (never on native apps until §19 gap 6 lands), **Google** always. For a returning person whose last method was a passkey (remembered on the device, not the identity), the heading becomes "Welcome back." and **Sign in with your passkey** is the primary button, with GitHub second.
- **From the terminal** (`?next=/device?user_code=…`): a line under the heading, `--cloud-mint-wash` background: "Your terminal is waiting. Sign in, then approve it."
- **States and copy**:
  - GitHub busy: the button reads "Opening GitHub…" with a spinner.
  - Provider came back without signing in: "GitHub didn't sign you in: the request was cancelled. Try again, or use another way."
  - Invalid email: under the field, bad colour with ✕: "That doesn't look like an email address."
  - Rate limited (the auth layer's limits): "Too many tries from this network. Try again in 12 minutes."
  - An account with this email signs in another way (`merge_offer`): "ana@example.com already has an account that signs in with GitHub. Continue with GitHub to add this way to it."
  - Offline: the buttons are disabled and one line says "You're offline. Signing in needs a connection."
- **Motion**: none beyond the press. The sheet is already on the desk.

### 7.2 `/sign-in/email`

The same sheet. "Check your email." (hand). "We sent a code to ana@example.com." One `TextInput().autocomplete("one-time-code")` labelled "Code from the email", **Continue** (primary), then quiet **Use a different email** and **Send another code** (disabled for 30 seconds with "Send another code in 0:24" as its label, so the countdown is read, not just seen).

- Wrong code: "That code didn't match. Use the newest email from LeptosKit Cloud."
- Expired: "That code has expired. Send another code."
- Success: to `next`, or `/welcome` for a new account, or the last organisation.

### 7.3 The first thirty seconds: `/welcome`

Sign-up creates the person's account and a personal organisation whose slug comes from the GitHub login or the email's local part (`ana`, then `ana-2` if taken). `/welcome` shows once, as a short sheet, and asks one question:

```text
┌ sheet ───────────────────────────────────────────┐
│ Hello, Ana.                         ← hand, 36px   │
│ Your projects live at cloud.leptoskit.dev/ana.    │
│ You're on Hobby: free, one project, and nothing   │
│ here can charge you until you add a card.         │
│ ───────────────────────────────────────────────── │
│ Do you have a leptoskit app already?              │
│                                                   │
│ [ Yes, ship it from my terminal ]▌                 │ → /ana/new?path=terminal
│ [ Not yet, ship a starter       ]                 │ → /ana/new?path=starter
│                                                   │
│ Look around first                                 │ → /ana (quiet)
└──────────────────────────────────────────────────┘
```

Second by second: 0, the account and organisation exist; about 3, they've read two sentences and answered; by 30, they've either pressed **Ship** on a starter and are watching its sheet write itself, or they're looking at two commands and a page that's listening for their terminal. Nothing else is asked: no team name, no plan picker, no tour, no passkey prompt. "Add a passkey for one-touch sign-in" waits in **To do** until the first release is live.

People who arrived from `lk cloud login` skip `/welcome`: they go straight to `/device`, whose sheet starts with "Your account is ready." so the step isn't lost.

### 7.4 `/device`: approve a terminal

**Purpose**: let the person who ran `lk cloud login` approve it, and make approving someone else's code hard to do by accident.

```text
┌ sheet, max-w-md ──────────────────────────────────────┐
│ CONNECT A TERMINAL                         ← group label│
│ Does your terminal show this code?                     │
│                                                        │
│            WDJB-MJHT              ← mono, text-3xl      │
│                                                        │
│ ────────────────────────────────────────────────────── │
│ The leptoskit CLI (lk cloud login) asks to act         │
│ as Ana Lima (ana@example.com):                         │
│   • see your organisations and projects                │
│   • ship releases and change settings                  │
│   • publish to the stores you've connected             │
│ Started 40 seconds ago, near Berlin, Germany.          │ ← §19 gap 7
│ ────────────────────────────────────────────────────── │
│ Only approve a code you started yourself, just now.    │
│ [ Approve this terminal ]▌        Not me, deny          │
└────────────────────────────────────────────────────────┘
```

- **Without a code** (`/device`): one field, "Enter the code your terminal shows", that accepts lower case and a missing dash, and **Continue**.
- **Components**: sheet, `Text` for the code with `accessibility_label("W D J B, M J H T")`, `raised(Button)`, quiet deny, `Stamp`. Signed out, it redirects to `/sign-in?next=…` and comes back.
- **Operations**: `leptoskit_idp::client::use_device_request` (the app, its scopes and the code) and `decide_device`; the scope list is the idp's `ScopeDescription`s, which the cloud registers in plain words.
- **After approving**, an `APPROVED 14:01` stamp (good) lands on the sheet, then the sheet says: "Your terminal is signed in. Next, in the same terminal:" with a `CommandBlock` for `lk cloud launch`, and "This page will follow along." **Continue** (focused, so Enter is enough) goes to `/<org>/new?path=terminal` with the first item ticked (§8.2); if the person stays, the page goes there by itself when `lk cloud launch` creates the project, because that is what they are waiting for. It never moves on a timer. A person who already has projects sees "or `lk deploy` in a project you've launched before", and the shipping chip picks up whatever they ship.
- **Denied**: "Denied. Your terminal will say so. If you didn't start this, someone may have sent you the link; you don't need to do anything else."
- **Expired or used**: "This code has expired. Run `lk cloud login` again for a new one." / "This code was already used."
- **On a phone**: the CLI can print `verification_uri_complete` as a QR code in the terminal; scanning it opens this page in the cloud app (a universal link), where approving is one press.

## 8. New project: `/<org>/new`

**Purpose**: get from nothing to a live URL by whichever path fits.

```text
Desktop (lg and up): three equal cards in a row, grid-cols-3 gap-4

Ship an app                                                   ← hand, 36px
Three ways in. Each ends the same way: a signed release sheet and a live URL.

┌ From your terminal ─────────┐ ┌ A starter, in one press ────┐ ┌ From GitHub ────────────────┐
│ For an app you already have. │ │ No code yet? Ship one of     │ │ Ship a repository you        │
│                              │ │ ours and change it later.    │ │ already have on GitHub.      │
│ $ lk cloud login        Copy │ │ ◉ Postgres and sign-in       │ │                              │
│ $ lk cloud launch       Copy │ │   lk new --db postgres --auth │ │ [ Connect GitHub ]           │
│                              │ │ ○ SQLite                     │ │                              │
│ We'll follow along here:     │ │   lk new --db sqlite         │ │ GitHub asks which            │
│ ● Approve your terminal      │ │ ○ No database                │ │ repositories we may read.    │
│ ○ Run lk cloud launch        │ │   lk new                     │ │ We read only those.          │
│                              │ │ Name  [ first-app      ]     │ │                              │
│                              │ │ Free on Hobby; sleeps when   │ │                              │
│                              │ │ idle.                        │ │                              │
│                              │ │ [ Ship first-app ]▌          │ │                              │
└──────────────────────────────┘ └──────────────────────────────┘ └──────────────────────────────┘

Phone: one column; the starter card first (the only path that works without a computer), then
GitHub, then the terminal card titled "On your computer". `?path=` puts that card first and
collapses the other two to one-line rows ("Or: from your terminal ›").
```

- **Components**: `TurnCard` fronts for the three cards (each has a back: the terminal card's back is the `[cloud]` table launch will write; the starter's back is the template's `leptoskit.toml`; GitHub's back is the permissions the GitHub App asks for), `CommandBlock`, `ListeningChecklist`, `ChoiceRow` for starters, `TextInput` for the name (validated against the slug rule as you type: "Starts with a letter; letters, digits and dashes; 3 to 40 characters"), `raised(Button)`.
- **Key classes**: cards `CARD` + `min-h-[360px]`; the starter's command line under each choice in `CODE` at `text-xs text-[var(--cloud-ink-3)]`.
- **Operations**: `account::organizations`; `projects::projects` (live, to notice a new project); `projects::create_project`; `deployments::deploy` with `template { name }`; `billing::estimate` for the starter's cost line; `deployments::deployments` and `deployment_events` (live) once a release exists.

### 8.1 The starter path

The starters are the `lk new` templates the cloud offers (contract `template { name }`), at most three, each shown with the command that makes it, so the person learns the CLI while skipping it. **Ship first-app** creates the project and its production environment, starts a release from the template, and moves to `/<org>/first-app/production/1`, where the sheet writes itself (§9) already signed: "Signed off by Ana Lima, from the web". From press to sheet is one round trip; there is no confirm step, because nothing it does costs money on Hobby or touches anything that exists.

When it's live, the sheet's **What to do next** starts with **Get the code**: `lk cloud project clone first-app`, which downloads the source the cloud generated and writes `[cloud] project = "first-app"`, so `lk deploy` works from there. (The contract has no clone yet: §20.)

### 8.2 The terminal path, and the page coming alive

The terminal card listens from the moment the page opens. It reacts only to what the API actually sees, so each step it shows is real:

| The terminal prints                                                                   | The page does                                                                                                                                                                                                                      |
| ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lk cloud login` opens the browser                                                    | `/device` (§7.4); after approval the card's first item ticks: "✓ Terminal signed in: ana-mbp, lk 0.9.0"                                                                                                                            |
| `lk cloud launch` lists what it found and the monthly estimate, then "Ship it? [Y/n]" | the second item pulses ("Run `lk cloud launch` in your project"); the API hasn't seen anything yet, so the page says nothing it doesn't know                                                                                       |
| "Created project notes (production)"                                                  | **the card turns into the release sheet in place**: the header writes `notes · production · v1`; the lines write in one after another (Server, Database, each service, Web assets, Over the air), then the cost and the fine print |
| "Release v1 queued"                                                                   | the signature writes itself: "Signed off by Ana Lima, from the terminal on ana-mbp"; the stage rail lights `Queued`                                                                                                                |
| "Building…"                                                                           | the rail moves to `Building` with a running clock; the right column opens on the build log, streaming; the Server line shows "Building · step 3 of 6 · cargo build"                                                                |
| "Built in 4m 12s"                                                                     | `BUILT` lands on Server and Web assets                                                                                                                                                                                             |
| "Migrated 1 migration"                                                                | `MIGRATED` lands on Database                                                                                                                                                                                                       |
| "Live: https://notes-ana.leptoskit.app"                                               | `ROLLED OUT` on Server, then the sheet's big `LIVE` stamp lands across the signature; the URL appears under it in mono 20px with **Open ↗** and **Copy**                                                                           |
| "What to do next: …"                                                                  | the same list appears under the sheet (§9.9)                                                                                                                                                                                       |

The "turns into" is the card's `TurnCard` front being replaced by the `ReleaseSheet` with the write motion; no route change until the sheet exists, then the URL becomes `/<org>/notes/production/1` with `replace` so Back returns to `/new`.

For the first two rows to be richer (the lines written before the build, the `.env` names launch noticed), `create_project` has to carry what the CLI found. §20 asks for that; until then the lines appear when the release's plan arrives from `deploy`.

- **Waiting a long time** (10 minutes, nothing): the second item stops pulsing and a margin note says: "Still waiting. If `lk cloud launch` printed an error, it says what to fix. Check its first line too: it names the account it's signed in as."
- **First build**: a margin note beside the rail: "The first build has nothing cached, so it's the slowest one you'll see."

### 8.3 From GitHub

**Connect GitHub** goes to GitHub's app installation, where the person picks repositories, and comes back to `/<org>/new/github`: a filterable list (repository, private or public, last push), then the branch, then a **draft sheet** built from that branch's `leptoskit.toml` with its cost, fine print and a signature line. Signing ships `git { repository, commit }` at the branch head. Shipping on every push is a project setting for later, not part of the first release.

- No repositories granted: "GitHub gave us no repositories. Choose some on GitHub ↗"
- No `leptoskit.toml` at the root of that branch: "`acme/site` has no `leptoskit.toml` at its root on `main`."
- The file doesn't validate: the validation error with its line, in a `paper_code` block, and "Fix it on GitHub, then press Check again."

None of this has operations in the contract yet (§20); build it after the other two paths.

## 9. The release sheet: `/<org>/<project>/<env>/<n>`

**Purpose**: one page that is the plan before a release, its timeline while it ships, and its record afterwards.

### 9.1 Layout

```text
Desktop, ≥ 1280px, with the right column open on the build log (content in 6 columns)

┌ content ──────────────────────────────────────────────────────┐ ┌ right column, w-[440px] ──────────┐
│ ‹ Releases                                                    │ │ Events   Build log   Runtime log   │
│ ┌ sheet ───────────────────────────────────────── [Workings] ┐│ │ ───────────────────────────────── │
│ │ RELEASE                               notes · [production] ││ │ 14:02:40 cargo build --release    │
│ │ v42                                                        ││ │   Compiling notes_server v0.4.0   │
│ │ 3f9c2e1  Add tags to notes                                 ││ │   Compiling notes_routes v0.4.0   │
│ │                                                            ││ │ …                                 │
│ │  ●────────●────────◐────────○────────○                     ││ │                                   │
│ │  Queued   Building Migrating Rolling out Live              ││ │                                   │
│ │  2s       4m 12s   0:03                                    ││ │ ● Following            Pause      │
│ │ ────────────────────────────────────────────────────────── ││ └───────────────────────────────────┘
│ │┃ WHAT SHIPS           WHERE                A MONTH         ││
│ │┃ Server               eu-central           $4.00–$12.00  BUILT
│ │┃  image 9a1f…c3 · 1–3 × s                                  ││
│ │┃ Service: search      eu-central           ≈ $0.24       BUILT
│ │┃  m · sleeps when idle                                     ││
│ │┃ Database             notes (shared)       included      ◐ Migrating
│ │┃  0007_add_tags                                            ││
│ │┃ Web assets           edge                 —             BUILT
│ │┃  412 kB · old assets kept for open tabs                   ││
│ │┃ Over the air: iOS    1,204 devices on store build 41      ○
│ │┃ Over the air: Android  873 devices on store build 43      ○
│ │ ────────────────────────────────────────────────────────── ││
│ │ After this release  $4.24 – $12.24 a month · cap $20 ✓ fits ││
│ │ This release's builds  6.1 Linux minutes · $0.03            ││
│ │ ────────────────────────────────────────────────────────── ││
│ │ WHAT THIS RELEASE WON'T DO                                  ││
│ │ • A rollback won't undo migration 0007_add_tags.            ││
│ │ • iOS devices on store build 38 need build 41 from the App  ││
│ │   Store before they get this.                               ││
│ │ • search sleeps when idle; its first request waits while it ││
│ │   starts (1.2s last time).                                  ││
│ │ ────────────────────────────────────────────────────────── ││
│ │ Signed off by  Ana Lima                    ← pen, hand 30px ││
│ │ from the terminal on ana-mbp · 14:02:11                     ││
│ │                                        Stop building v42    ││
│ └────────────────────────────────────────────────────────────┘│
└───────────────────────────────────────────────────────────────┘
  ┃ is the ember margin rule. The stamp column is the right edge of each line.
  When live, a large LIVE stamp lands at the bottom right, over the signature block,
  with "14:06:40 · eu-central" under the word and the URL beneath the sheet.
```

```text
Phone

┌──────────────────────────────┐
│ ‹ notes          [production] │
├──────────────────────────────┤
│ RELEASE             Workings  │
│ v42                           │
│ 3f9c2e1 Add tags to notes     │
│ Migrating · 0:03              │  ← the current stage in words
│ ● ● ◐ ○ ○                     │  ← dots only, labels in accessibility_label
│ ──────────────────────────── │
│ Server                 BUILT  │
│ eu-central · 1–3 × s          │
│ $4.00–$12.00 a month          │
│ ──────────────────────────── │
│ Database          ◐ Migrating │
│ notes (shared) · 0007_add_tags│
│ …                             │
│ ──────────────────────────── │
│ $4.24–$12.24 a month · cap ✓  │
│ ▸ What this release won't do (3)
│ ──────────────────────────── │
│ Signed off by Ana Lima        │
│ terminal on ana-mbp · 14:02   │
│ [ Stop building v42 ]         │
│ ──────────────────────────── │
│ Events   Build log   Runtime  │  ← the right column, as a section
│ …                             │
└──────────────────────────────┘
```

Lines appear in a fixed order: Server, each service, Database (with its migrations), Web assets, Over the air per platform, then each store. A line only exists when the release touches that target. On phones the fine print collapses to a disclosure (`aria_expanded`) after the first item.

### 9.2 Anatomy and classes

These go in `leptoskit_cloud_app::style`.

| Piece          | Classes                                                                                                                                                                                                                                                                                  | Notes                                                                                           |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `SHEET`        | `relative flex-col gap-5 rounded-[6px] border border-[var(--cloud-line-strong)] bg-[var(--cloud-sheet)] px-5 py-6 md:px-8 md:py-8`                                                                                                                                                       | inside a `relative` wrapper whose first child is `SHEET_BLOCK`                                  |
| `SHEET_BLOCK`  | `absolute inset-0 translate-x-1.5 translate-y-1.5 rounded-[6px] bg-[var(--cloud-block)]`                                                                                                                                                                                                 | `aria_hidden(true)`                                                                             |
| `SHEET_RULE`   | `h-px w-full bg-[var(--cloud-rule)]`                                                                                                                                                                                                                                                     | between sections, and under each line                                                           |
| `SHEET_MARGIN` | `absolute top-0 bottom-0 left-3 md:left-5 w-px bg-[var(--cloud-ember)]/40`                                                                                                                                                                                                               | the notebook margin; decorative, `aria_hidden(true)`                                            |
| `SHEET_NUMBER` | `text-4xl font-semibold leading-none tabular-nums text-[var(--cloud-ink)]`                                                                                                                                                                                                               | `v42`                                                                                           |
| `LINE`         | `flex-row flex-wrap items-start gap-x-3 gap-y-1 min-h-12 py-3`                                                                                                                                                                                                                           | phones: title and stamp on one row, the rest below                                              |
| `LINE_TITLE`   | `text-sm font-semibold text-[var(--cloud-ink)]`                                                                                                                                                                                                                                          |                                                                                                 |
| `LINE_DETAIL`  | `text-xs text-[var(--cloud-ink-3)]`                                                                                                                                                                                                                                                      | digests shortened `9a1f…c3`, copyable from the back                                             |
| `LINE_COST`    | `w-[132px] text-end text-sm tabular-nums text-[var(--cloud-ink-2)]`                                                                                                                                                                                                                      | full width, start-aligned on phones                                                             |
| `STAMP`        | `flex-row items-center rounded-[4px] border-2 px-1.5 py-0.5` + tone + rotation                                                                                                                                                                                                           | tone: `border-[var(--cloud-good)] text-[var(--cloud-good)] bg-[var(--cloud-good)]/5`, and so on |
| `STAMP_LARGE`  | `flex-col items-center gap-1 rounded-[6px] border-[3px] px-3 py-2` + tone + rotation                                                                                                                                                                                                     | the word at `text-xl font-bold`, then time and place at 11px                                    |
| `SIGNATURE`    | `font-[var(--cloud-font-hand)] text-[30px] leading-none text-[var(--cloud-pen)]`                                                                                                                                                                                                         | the "via" line under it in the label style (`text-xs font-medium text-[var(--cloud-ink-3)]`)    |
| `STATION_DOT`  | `h-3 w-3 rounded-full` + done `bg-[var(--cloud-ink)]`, current `border-2 border-[var(--cloud-ember)] bg-[var(--cloud-sheet)] animate-pulse`, pending `border-2 border-[var(--cloud-control-line)]`, skipped `border-2 border-[var(--cloud-line-strong)]`, failed `bg-[var(--cloud-bad)]` | each dot also has a glyph in its label: ✓, ●, ○, –, ✕                                           |
| `STATION_LINK` | `mt-[5px] h-0.5 flex-1` + `bg-[var(--cloud-ink)]` behind done stations, `bg-[var(--cloud-line-strong)]` ahead                                                                                                                                                                            |                                                                                                 |

**The unsigned signature line** (draft sheets): "Sign here" in the label style (`text-xs font-medium text-[var(--cloud-ink-3)]`), then a dashed rule drawn as a `Vector` path (`Stroke::dashes(&[2.0, 4.0], 0.0)` in `--cloud-control-line`; §19 gap 8), then **Sign and ship v44** (primary, raised) and, under it, one sentence of consequence: "This ships to production now." Pressing it disables the button ("Shipping…"), sends the mutation with an idempotency key, and when the release exists the signature writes itself with the person's name. The draft sheet never ships offline: the button is disabled while `use_live_status()` isn't `Live`, with "Reconnect to ship" as its description, and these mutations don't use the offline queue.

**Who signed**: "from the web", "from the terminal on ana-mbp" (the CLI session's device name), or "by token `ci-deploy`, created by Ben Ode" for API tokens. The signer is recorded by the server from the caller's identity, never from an argument.

### 9.3 States

| State                     | Stage rail                         | Lines                                                                    | Bottom of the sheet                                                    | Actions                                | Announced (`aria_live`)                                   |
| ------------------------- | ---------------------------------- | ------------------------------------------------------------------------ | ---------------------------------------------------------------------- | -------------------------------------- | --------------------------------------------------------- |
| draft                     | all pending                        | planned, no stamps                                                       | the unsigned signature line                                            | **Sign and ship v44**, **Discard**     |                                                           |
| signing                   | all pending                        | planned                                                                  | button busy, "Shipping…"                                               |                                        | "Shipping v44" (polite)                                   |
| queued                    | `Queued` current, elapsed          | planned                                                                  | signature                                                              | **Stop v42** (quiet danger)            | "v42 queued"                                              |
| building                  | `Building` current                 | Server, services, web in progress ("step 3 of 6 · cargo build")          | signature                                                              | **Stop building v42**                  | "v42 building"                                            |
| migrating                 | `Migrating` current                | Database in progress, migration name                                     | signature; "Can't stop now: migrations are running."                   |                                        |                                                           |
| rolling out               | `Rolling out` current              | Server "2 of 3 ready"; others stamped                                    | signature                                                              |                                        |                                                           |
| live                      | all done, durations                | every line stamped                                                       | large `LIVE` stamp; the URL with **Open ↗** and **Copy**               | **Roll back to v41…**                  | "v42 is live" (polite)                                    |
| superseded                | all done                           | stamps at `opacity-60`                                                   | large `SUPERSEDED` stamp, "by v43 at 15:10" (a link)                   | **Ship v42 again** (a rollback to v42) |                                                           |
| failed { stage, blocker } | the failed station ✕, later ones – | the failing line `FAILED` with one line of reason; later lines `SKIPPED` | large `FAILED` stamp and the `BlockerPanel` (§9.4); "Serving now: v41" | the blocker's actions                  | "v42 failed while building: the build failed" (assertive) |
| cancelled                 | the station it stopped at, –       | lines not reached `SKIPPED`                                              | large `CANCELLED` stamp, "by Ana Lima at 14:03"                        | **Ship v42 again**                     | "v42 stopped"                                             |
| unknown                   | the station it was in, "?"         | the line in progress `UNKNOWN`                                           | the unknown panel (last row of §9.4)                                   | none but **Open events**               | polite                                                    |

A rollback's sheet differs in its lines, which is where honesty matters most:

- the header says "v44 · rollback of v41";
- **Server**: "image from v41 · nothing to build", and the `Building` station is skipped with "rollback";
- **Database**: "No change. A rollback doesn't touch the database: v42's migration `0007_add_tags` stays." The line is stamped `SKIPPED`;
- **Over the air**: "Devices go back to v41's interface.";
- **Stores**: "Store builds aren't recalled. Play keeps build 44 on the internal track." (stamped `SKIPPED`);
- the fine print adds: "If v41 can't read data v42 wrote, roll forward instead: fix it and ship v45."

### 9.4 Failure: blockers and what to do next

The failing line carries a `FAILED` stamp and one line of reason. The bottom of the sheet carries the `BlockerPanel`: a `bg-[var(--cloud-bad)]/10` panel with ✕ and a headline, the reason with real detail, a "Serving now: v41" fact, and the actions. Actions a role can't take are shown disabled, with who can take them named in the description. The reason sentence comes from the API's error detail; the copy below is the template.

| Blocker               | Stage       | Headline                                                | Why (example detail)                                                                                                                                                     | Next action                                                                                                                  |
| --------------------- | ----------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `build_failed`        | building    | The build failed.                                       | `cargo build` stopped at `src/routes/notes.rs:42:17`: expected `String`, found `&str`. (The first error in the build log.)                                               | **Open the build log at the error** · **Copy the error for an agent** · "Fix it and run `lk deploy`."                        |
| `builder_unavailable` | queued      | No build machine could take this release.               | Every builder in eu-central was busy or offline for 15 minutes. Nothing changed in production.                                                                           | **Ship v42 again**                                                                                                           |
| `schema_incompatible` | migrating   | v41 can't run on the database this release wants.       | Migration `0007` drops `notes.body`, which v41 still reads. During a rollout both versions run, so v41 would break. Nothing was changed.                                 | "Ship it in two releases: first stop reading `body`, then drop it." **Read: changing a schema without downtime ↗**           |
| `migration_failed`    | migrating   | Migration `0007_add_tags` failed.                       | Postgres said: `column "tags" of relation "notes" already exists`. v41 is still serving. The migration log shows each statement that ran.                                | **Open the migration log** · "Fix the migration, then ship again."                                                           |
| `not_ready`           | rolling out | The new version never became ready.                     | `/healthz` answered 503 for 5 minutes on 2 of 2 new processes. v41 served the whole time.                                                                                | **Open the new processes' logs** (runtime logs filtered to v42) · **Ship v42 again**                                         |
| `quota_exceeded`      | queued      | This release needs more than Hobby allows.              | It asks for size `s`; Hobby runs size `xs` only.                                                                                                                         | **Move to Pro** · "Or set `size = "xs"` under `[cloud.environments.production]` and ship again."                             |
| `spend_cap_reached`   | queued      | You've reached this month's $20 spend cap.              | New releases wait until the cap goes up or the month ends on 1 December. v41 keeps serving.                                                                              | **Raise the cap** · for other roles: "Only owners and billing members can raise it: Ana Lima, Ben Ode."                      |
| `payment_required`    | queued      | The last payment didn't go through.                     | Stripe couldn't charge the card ending 4242 on 1 November.                                                                                                               | **Update the payment method ↗** (the Stripe portal)                                                                          |
| `cluster_unavailable` | any         | The eu-central cluster isn't reachable.                 | Its worker stopped checking in at 14:01. The cluster keeps running what it had (v41) on its own, but we can't confirm that until it's back.                              | **Ship v42 again** (enabled when the cluster reconnects) · **Status ↗**                                                      |
| `interrupted`         | any         | The release was interrupted. We checked what's running. | The worker stopped during the rollout. The cluster shows v41 serving and none of v42.                                                                                    | **Ship v42 again**                                                                                                           |
| `secret_unavailable`  | rolling out | A secret this release needs couldn't be read.           | `STRIPE_KEY` was removed after the build. v41 is still serving.                                                                                                          | "Set `STRIPE_KEY` again, then ship again"                                                                                    |
| none yet: unknown     | any         | We don't know yet whether v42 shipped.                  | We lost contact with the worker during the rollout at 14:05. Before calling it failed, we're checking what the cluster is actually running. This can take a few minutes. | none; **Open events**. The contract's rule: an expired lease isn't proof the old process stopped, so the page never guesses. |

**Copy the error for an agent** copies a short Markdown block: the release, the blocker code, the first error with file and line, and the command to reproduce (`cargo build --release -p notes_server`). It's the agent-first idea at the moment it's most useful.

### 9.5 Cancel, roll back, ship again

- **Cancel** exists while a release is queued or building (the contract's rule). **Stop building v42** is a quiet danger button; pressing it expands a `ConfirmPanel` in place: "Stop v42? The build stops and nothing changes in production." **Stop v42** / **Keep building**. Then `cancel_deployment`, and the `CANCELLED` stamp.
- **Roll back** is signing a new sheet. **Roll back to v41…** (overview, live sheets) and **Ship v41 again** (superseded sheets) open `/<org>/<project>/<env>/ship?from=41`, a draft rollback sheet with the lines in §9.3, then `rollback`.
- **Ship again** (after a failure or cancel) opens `/<org>/<project>/<env>/ship?again=42`, a draft with the same source, then `deploy`. An upload source can be shipped again while the archive is kept; when it isn't, the draft says "This release's upload has expired; run `lk deploy` again."

Draft sheets need the plan (lines, costs, fine print) without shipping anything; §20 asks for a `preview_release` operation.

### 9.6 Workings: the back of the sheet

The mint tab in the sheet's corner (or `W`) turns the sheet over. Its back is a page on the ink underside with the admin's five-station rail:

| Station     | First sentence                                                             | What it shows                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Copy as                     |
| ----------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------- |
| **Declare** | "The parts of `leptoskit.toml` that shaped this release."                  | `[cloud]`, `[cloud.environments.production]`, `[db]`, `[services.*]`, `[app] platforms` and `[publish]`, with lines changed since v41 washed mint and marked `~`; then "Not from the file": scale set in the console, marked as differing                                                                                                                                                                                                                                                         | TOML                        |
| **Build**   | "What the build machine did, in order."                                    | each step with time and cache: fetch source `sha256:…` (2.1 MB), toolchain, `cargo build --release -p notes_server` (4m 02s, 78% cached), `leptoskit build --platform web` (38s), image `sha256:9a1f…` (64 MB), migrations found, and the **native fingerprint** with its decision: "Native inputs (Info.plist, AndroidManifest.xml, native plugins, host features) match store build 41, so this ships over the air. If they'd changed, the over-the-air lines would say 'needs a store build'." | `lk cloud logs --build 42`  |
| **Spec**    | "Everything production should be, as data. The cluster gets exactly this." | the `ReleaseSpec` as JSON: processes, images by digest, routes, database bindings, policy; secrets as names and versions (`SMTP_PASSWORD@v3`), never values                                                                                                                                                                                                                                                                                                                                       | JSON                        |
| **Apply**   | "What changed in the cluster, and how the rollout went."                   | Kubernetes objects with action words: `~ update Deployment notes-app (image v41 → v42)`, `= unchanged Service notes-app`, `+ create Job notes-migrate-42`, `~ update HTTPScaledObject notes-search`; then rollout events ("3 new processes ready; old ones stopped at 14:06:38")                                                                                                                                                                                                                  | YAML                        |
| **Cost**    | "How the monthly number on the front was worked out."                      | `size s: 0.25–1 CPU, 256 MiB · $4.00 for a month always on, billed by the second`, `× 1 always on = $4.00`, `+ up to 2 more under load: up to $8.00`, `search: size m ($8.00 a month), asleep 97% of last month: ≈ $0.24`, `builds: 6.1 Linux minutes × $0.005 = $0.03`; "Prices as of 18 November 2026."                                                                                                                                                                                         | `lk cloud usage --estimate` |

Action words use symbols and words as the admin's plan diffs do (`+ create`, `~ update`, `− delete`, `= unchanged`), never colour alone. Each code block is the admin's `code_block` with Copy.

**Full workings** for a card (`?workings=<card>`) open in the right column at `xl`, below the content elsewhere, and as a page on phones. They show all stations, where a card's back shows three to six lines.

### 9.7 Logs

Three sources, one `LogView`:

- **Events** (`deployment_events`, live): one line per state change and step, the clearest record of what happened, with time and duration.
- **Build log** (`build_logs`): the build's output, ANSI codes stripped, with line numbers. Lines that start an error (`error`, `error[E…]`) get a `--cloud-bad` wash and ✕ in the gutter, and **Jump to the first error** sits above the log.
- **Runtime log** (`runtime_logs`; also its own page `/<org>/<project>/<env>/logs`): time, process and a short pod name (`app·7f9c`), then the message. Filters: process (all, `app`, each service), a time range (15 minutes, an hour, a day, the retention the API reports), and a text filter.

```text
┌ LogView ──────────────────────────────────────────────┐
│ [all processes ▾] [last hour ▾] [ filter text      ]  │  ← ChoiceRow, ChoiceRow, TextInput
│ 14:05:02  app·7f9c    GET /notes 200 12ms             │  ← mono 13px; time ink-3; process ink-2
│ 14:05:03  search·2a1  woke in 1.2s                    │
│ 14:05:09  app·7f9c    ✕ panicked at src/db.rs:88 …    │  ← bad wash on error lines
│ ● Following                                    Pause  │
└───────────────────────────────────────────────────────┘
```

- **Following**: new lines arrive at the bottom and the view stays at the end (`scroll_offset`). Scrolling up pauses it: "Paused · 212 new lines" becomes a button that jumps to the end and follows again.
- Lines wrap; nothing scrolls sideways. A word longer than the row (a digest, a URL) overflows on native, where labels break only between words (§19 gap 5).
- Log text is untrusted: rendered as plain `Text`, never parsed as markup.
- `VirtualList` renders only the visible lines.
- Empty: "The build log starts when a builder picks this release up." / "No lines in the last 15 minutes. `search` is asleep, so it isn't writing any." Retention: "Runtime logs are kept for 7 days." with the number the API returns.

### 9.8 Release history: `/<org>/<project>/releases`

A list of sheet strips, newest first:

```text
┌──────────────────────────────────────────────────────────────────────────────────────┐
│▌v42  [production]  3f9c2e1 Add tags to notes          Ana Lima   ✓ ✓ ✓ ✓ • •   2h · 4m 30s │ ← serving: 2px ember bar, "Serving"
│ v41  [production]  rollback of v39                    token ci   ✓ – ✓ ✓        1d · 41s    │
│ v40  [production]  8d21aa0 Upgrade sqlx               Ben Ode    ✕ – – –        1d · 2m    │
│ v12  [staging]     8d21aa0 Upgrade sqlx               Ben Ode    ✓ ✓ ✓ ✓        1d · 4m    │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

- The mini stamps are one per line of the sheet (16px rounded squares with ✓, !, ✕, •, – and the target's icon in their label: "Server built, Database migrated…").
- Filters: the environment row, and **Failed only** (`Toggle`). **Show older** loads the next page.
- Phones: two lines per release (number, environment and source; then signer, stamps and age).
- Empty: "No releases yet. Run `lk deploy` in your project, or ship a starter." with both actions.

### 9.9 What to do next, after the first release

Under the first live sheet, a `StepList`: **Get the code** (starters), **Add a domain**, **Set the secrets `lk cloud launch` noticed** (names from `.env`, never values; "We didn't upload the file"), **Connect a store** (when `[publish]` names one), **Add a passkey**. Each step has one action and disappears when done; **Hide this** dismisses the list.

### 9.10 Operations, motion, accessibility

- **Operations**: `deployments::deployment`; `deployments::deployments` and `deployment_events` (live); `build_logs`; `runtime_logs`; `deploy`, `rollback` and `cancel_deployment` with idempotency keys; `billing::estimate` for draft totals; `projects::environments` for the header.
- **Motion**: lines write in (90ms stagger) the first time a sheet is seen in a state with new lines; stamps land once, when their event arrives, never on reload (a stamp already in the data renders at rest); the signature writes once when the release is created; reduced motion shows everything at rest. Pulsing stops when a station isn't current.
- **Accessibility**: the sheet is a labelled region ("Release v42 of notes in production"); lines are a list; each stamp's label is a sentence ("Server: built at 14:06"); the stage rail is a list with `aria_current` on the current station; one visually hidden live region announces stage changes politely and failures assertively; the signature's label is "Signed off by Ana Lima, from the terminal on ana-mbp, at 14:02". Focus order: back link, Workings tab, lines (each focusable when it has a Workings back), totals, fine print, signature area actions, then the log tabs.

## 10. Projects and the project overview

### 10.1 Projects: `/<org>`

**Purpose**: see every app in the organisation and whether each is fine, in one glance.

```text
Desktop                                                                     Phone
Projects                                         [ + New project ]          ┌──────────────────────────────┐
┌ notes ─────────────────────────────────────────────────────────────┐      │ Apps               + New      │
│ notes-ana.leptoskit.app                                    $1.84    │      ├──────────────────────────────┤
│ [production] ✓ v42 Live 2h    [staging] • Asleep    [pr-81] ✓ v3    │      │ notes                 $1.84  │
└─────────────────────────────────────────────────────────────────────┘      │ ✓ production v42 · 2h        │
┌ blog ──────────────────────────────────────────────────────────────┐      │ • staging asleep              │
│ blog.acme.dev                                              $0.12    │      ├──────────────────────────────┤
│ [production] ● v7 Building 1:12                                     │      │ blog                  $0.12  │
└─────────────────────────────────────────────────────────────────────┘      │ ● production v7 building     │
                                                                            └──────────────────────────────┘
```

- One card per project, `CARD` with `gap-2`: the name (link to the overview), the production URL in mono, this month's cost so far at the end (`tabular-nums`), and a wrapping row with one entry per environment: its pill, then the status word for what's serving (`✓ v42 Live 2h`, `• Asleep`, `● v7 Building 1:12`, `✕ v40 Failed`, `! Cap reached`).
- Each card's back: the namespaces, one per environment, with their quota and the node pool.
- **Operations**: `projects::projects` (live); the state per environment needs the org-wide in-flight releases and environment status queries in §20.
- **Empty**: a sheet with "Ship your first app" (hand) and the three paths of §8 as three rows; Hobby's line: "Hobby runs one project for free."
- **Hobby with one project**: **New project** is disabled with "Hobby runs one project. Pro runs as many as you like." and **See Pro** beside it.

### 10.2 Overview: `/<org>/<project>?env=<env>`

**Purpose**: answer "is it up, what's serving, what's it costing" without scrolling, and show the shape of the app.

```text
Desktop (xl), 12 columns
notes                                                                    notes-ana.leptoskit.app  Open ↗  Copy
[production]  [staging]  [pr-81]                                         + Environment
┌ Now serving ─────────────────────────────────── 8 cols ┐ ┌ This month ──────────── 4 cols ┐
│ v42 · 3f9c2e1 Add tags to notes          ✓ Live 2h      │ │ $1.84 so far                    │
│ BUILT MIGRATED ROLLED OUT LIVE   (mini stamps)          │ │ about $4.10 by 30 November      │
│ Signed off by Ana Lima · terminal on ana-mbp            │ │ ▓▓▓▓░░░░░░░░░░░░░░░░ cap $20     │
│ Open the sheet   Roll back to v41…                      │ │ updated 3 min ago   Details →   │
└─────────────────────────────────────────────────────────┘ └─────────────────────────────────┘
┌ How it fits together ────────────────────────── 8 cols ┐ ┌ Health, last 24 hours ─ 4 cols ┐
│  Web ──┐                                                │ │ ✓ app  2 of 2 ready             │
│         ├─▶ [ app  1–3 × s  ✓ 2 running ] ──▶ [ Database │ │ • search asleep · woke 41 times │
│  iOS ───┤        │                              shared   │ │   median wake 1.2s              │
│  1,204  │        └─▶ [ search  m  • asleep ]    212 MB ] │ │ ✓ 0 restarts · 0 out of memory  │
│  Android┘                                               │ │ Request rates live in your     │
│  873                                                    │ │ app's own admin.               │
└─────────────────────────────────────────────────────────┘ └─────────────────────────────────┘
┌ Recent releases ──────────────────────────────────────────────────────────── 12 cols ┐
│ v42 … (strips, §9.8, the last five)                                    All releases → │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

```text
Phone
┌──────────────────────────────┐
│ ◰ notes         [production] │
├──────────────────────────────┤
│ notes-ana.leptoskit.app      │
│ [ Open ↗ ]  [ Copy ]         │
│ [production] [staging] [pr-81]│
│ ┌ Now serving ─────────────┐ │
│ │ v42 ✓ Live 2h            │ │
│ │ ✓ ✓ ✓ ✓  Ana Lima        │ │
│ └──────────────────────────┘ │
│ ┌ This month ──────────────┐ │
│ │ $1.84 so far · ~$4.10    │ │
│ │ ▓▓▓░░░░░░░░░ cap $20      │ │
│ └──────────────────────────┘ │
│ ┌ How it fits together ────┐ │
│ │ Web, iOS 1,204, Android  │ │
│ │   │                      │ │
│ │ app 1–3 × s ✓ 2 running  │ │
│ │   ├── search m • asleep  │ │
│ │   │                      │ │
│ │ Database shared 212 MB   │ │
│ └──────────────────────────┘ │
│ Health ✓ 2 of 2 · 0 restarts │
│ IN THIS PROJECT              │
│ Releases                   › │
│ Logs                       › │
│ Scale                      › │
│ Variables and secrets      › │
│ Domains                    › │
│ Database                   › │
│ Stores                     › │
│ Settings                   › │
└──────────────────────────────┘
```

**Cards**:

- **Now serving**: `TurnCard`. The front is the serving sheet as a strip: number, source, status word with age, its mini stamps, the signature line in the label style, and **Open the sheet** and **Roll back to v41…**. While a release is in flight, a second row appears above it, "Shipping v43 · Building 1:12", linking to its sheet. The back: the Deployment, its ReplicaSet and pods (names, image digest, age, restarts).
- **This month**: `CostMeter` (§11.4). The back: the arithmetic so far.
- **How it fits together**: `Topology`. A fixed layout, never pan and zoom: clients in the first column (web, then each platform with its device count for over-the-air updates), processes in the middle (the app, then services), data on the right (the database, object storage when used). Connectors are 1px `View`s with a `Vector` arrowhead. Each node is a `Pressable` to its page (a process to its logs, the database to `/database`, a platform to Stores) and turns over to its Kubernetes objects. On phones the columns become rows, top to bottom, with the connectors vertical.
- **Health**: one row per process with what the runtime reports: ready of desired, restarts, sleeps and wake times, out-of-memory kills, each with an icon and a word. It never shows request charts, because the API has no metrics (§2, §20).
- **Recent releases**: the last five strips.
- **Environments** (phones: the pill row; desktop: the same row under the title): **+ Environment** expands an inline form: name, "Start from: production's settings", **Create staging**. `projects::create_environment`.

**Node and card states** (each with icon, word and sentence):

| State          | On the node                                      | Sentence (in the node's label and the Health row)                                                  |
| -------------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| running        | ✓ `2 running` (good)                             | "app: 2 of 2 ready."                                                                               |
| asleep         | • `asleep` (info), node border dashed (`Vector`) | "search is asleep. The next request wakes it; that took 1.2s last time."                           |
| waking         | ● `waking` with the running pulse                | "search is starting for a request."                                                                |
| rolling out    | ● `v41 → v42 · 2 of 3`                           | "Rolling out v42: 2 of 3 new processes ready."                                                     |
| crashing       | ✕ `restarting` (bad)                             | "app restarted 6 times in 10 minutes. The last exit was out of memory at 256 MiB." + **Open logs** |
| stopped at cap | ! `asleep: cap` (warn)                           | "Asleep because the spend cap was reached on 24 November." + **Raise the cap** (§11.5)             |
| unknown        | ? `not heard from` (warn)                        | "We haven't heard from eu-central since 14:01. This is what it reported then."                     |

**Page states**:

- **Loading**: skeleton cards (`--cloud-sunken`, no shimmer) in the final layout, so nothing jumps.
- **A new environment with nothing shipped**: Now serving reads "Nothing has shipped to staging yet." with `lk deploy --env staging` in a `CommandBlock`, and **Ship production's v42 here**.
- **Cap reached**: a `NOTE`-style panel above the cards, warn: "! You've reached this month's $20 spend cap. New releases wait. Environments that can sleep are asleep. Nothing has been deleted." **Raise the cap** (owners and billing members) or "Ask Ana Lima or Ben Ode to raise it."
- **Offline**: the `OfflineChip` in the top bar ("Offline · showing 14:02"); numbers keep their last values and their "updated" times keep counting.

- **Operations**: `projects::project`, `projects::environments`, `deployments::deployments` (live), `billing::usage`; runtime status and health need `environment_status` (§20).
- **Motion**: changed numbers wash mint; the shipping row enters with `Presence::fade().slide_by(0.0, -8.0)`; a waking node pulses. Reduced motion: no pulse, no slide; the word carries it.
- **Accessibility**: the topology is also a list ("How it fits together: Web and 2 platforms send requests to app, 2 of 2 running, which calls search, asleep, and uses the database, shared, 212 MB") as its `accessibility_label`, and each node is a link in reading order: clients, processes, data.

## 11. Scale and cost: `/<org>/<project>/<env>/scale`

**Purpose**: decide how big each process is, how many copies run, and whether it sleeps, with the price under your finger as you choose.

### 11.1 The price list the design uses

Prices are the catalogue's (contract §11); these are illustrative and the screens print whatever the catalogue holds:

| Size | CPU (request–limit) | Memory  | A month always on | Billed        |
| ---- | ------------------- | ------- | ----------------- | ------------- |
| `xs` | 0.1–0.5             | 128 MiB | $2.00             | by the second |
| `s`  | 0.25–1              | 256 MiB | $4.00             | by the second |
| `m`  | 0.5–2               | 512 MiB | $8.00             | by the second |
| `l`  | 1–2                 | 1 GiB   | $16.00            | by the second |
| `xl` | 2–4                 | 2 GiB   | $32.00            | by the second |

Databases: `shared` included (Hobby up to 500 MB); `s` $5.00, `m` $10.00, `l` $20.00 a month, storage $0.20 a GB-month. Linux builds $0.005 a minute, macOS $0.04. An always-on `m` with an `s` database is $13.00 a month, inside the contract's research figure.

### 11.2 Layout

```text
Desktop: controls in 7 columns, the receipt in 5
Scale · production
┌ app ───────────────────────────────────────────────┐   ┌ Receipt ─────────────────── sheet ┐
│ Size                                               │   │ PRODUCTION, A MONTH                │
│ ┌────┐┌════┐┌────┐┌────┐┌────┐                     │   │ app      1–3 × s    $4.00–$12.00   │
│ │ xs ││ s  ││ m  ││ l  ││ xl │   ← SizePicker      │   │ search   m, sleeps  ≈ $0.24        │
│ │0.5 ││ 1  ││ 2  ││ 2  ││ 4  │     CPU limit       │   │ Database shared     included       │
│ │128M││256M││512M││1G  ││2G  │                     │   │ ───────────────────────────────── │
│ │ $2 ││ $4 ││ $8 ││$16 ││$32 │     a month each    │   │ Now      $4.24 – $12.24            │
│ └────┘└════┘└────┘└────┘└────┘                     │   │ After    $8.24 – $24.24            │
│ Always running  [ − ]  1  [ + ]                    │   │          +$4.00 – +$12.00          │
│ At most         [ − ]  3  [ + ]                    │   │ ───────────────────────────────── │
│ Sleep when idle [  off ]                           │   │ Cap $20: fits at the bottom of the │
│ Sleeping needs "Always running" at 0.              │   │ range. Near the top it would reach │
└────────────────────────────────────────────────────┘   │ the cap; then new releases wait    │
┌ search ────────────────────────────────────────────┐   │ and what can sleep sleeps.         │
│ … the same controls                                │   └────────────────────────────────────┘
└────────────────────────────────────────────────────┘
┌ ChangeSlip ─────────────────────────────────────────────────────────────────────────────────┐
│ 2 changes: app size s → m, at most 3 → 3.  +$4.00–$12.00 a month.   [ Apply to production ]▌│
│ This will differ from leptoskit.toml. Paste this so the next lk deploy keeps it:  Copy TOML │
└─────────────────────────────────────────────────────────────────────────────────────────────┘

Phone: one column. Each process is a section; the SizePicker is a 5-column grid of 56px-tall boxes
(price and size only, CPU and memory in a line under the grid for the selected size). The receipt
follows the controls; the ChangeSlip is the last block on the page, above the tab bar, and only
exists while something is pending.
```

- **SizePicker**: five `Pressable` boxes in a `ChoiceRow` (`role(Role::RadioGroup)`, arrow keys move). Each is `rounded-lg border border-[var(--cloud-control-line)] p-2 min-h-14`, the selected one `border-2 border-[var(--cloud-ink)] bg-[var(--cloud-surface)]` with a ✓ in its corner, so selection never relies on colour. Locked sizes show a lock glyph and "Pro" and stay focusable, with "Hobby runs size xs. Pro runs every size." as their description.
- **Stepper**: `[ − ] value [ + ]`, 44px targets, the value in `text-lg tabular-nums`, `role(Role::SpinButton)` with `aria_value_now`, `aria_value_min` and `aria_value_max`; the arrow keys change it. At a limit the button disables and says why ("20 is the most for one process.").
- **Toggle** "Sleep when idle": the real `Switch` in a labelled row. On sets "Always running" to 0 and disables its stepper ("Sleeping means none run while idle"). Its description is the cold start in numbers: "The first request after sleeping waits while it starts: about a second; 1.2s last time."
- **Constraints shown where they bite** (contract §5):
  - live queries: the toggle is disabled with "Sleeping closes live connections, and notes uses live queries. To allow it anyway, set `allow_sleep_with_live = true` under `[cloud.environments.production]`.";
  - SQLite: "At most" is fixed at 1: "SQLite apps run exactly one copy. Postgres runs more.";
  - Hobby: `xs` only, at most 1, sleeping on: each locked control says so once, and the page has one **See Pro** link, not one per control.

### 11.3 The price as you move

Every press updates the receipt in the same frame: the receipt is computed in the client from the catalogue the page loaded (`billing::plans` returns the unit prices; §20 asks for that), then confirmed by `billing::estimate` 300ms after the last press. If the server's number differs, the server wins and the line washes mint. While the confirmation is in flight the receipt shows nothing different: the client number is already right for every catalogue price; the estimate catches what the client can't know (a discount, a plan change).

- **Ranges, not single numbers**: autoscaling makes the cost a range from "always running" to "at most", and a sleeping process is estimated from last month's awake share ("asleep 97% of last month"), or "≈ $0.10–$8.00, depending on traffic" for a new one.
- **Deltas in ink**: "+$4.00 – +$12.00" is ink, never red. Going down is "−$4.00", also ink.
- **The cap is the only alarm**: the cap line is good ✓ when the top of the range fits, warn ! when only the bottom fits ("Near the top it would reach the cap"), and bad ✕ when even the bottom doesn't ("This costs more than your $20 cap even at its smallest. Raise the cap or choose less.").

### 11.4 Applying, and "one file"

**Apply to production** sends `projects::update_environment` with the new scale; the worker runs a `scale` job. The slip stamps `APPLIED 14:20` and records who applied it ("Applied by Ana Lima"). It doesn't create a release: nothing was built.

Because `leptoskit.toml` is the source of truth (contract §5), the page is honest about the drift:

- after applying, the process card shows **differs from `leptoskit.toml`** (warn word) with the TOML to paste;
- the next release sheet from the file has a line "Scale: back to 1–3 × s from `leptoskit.toml`" so nobody is surprised;
- open question 1 (§22) is whether the console should change scale at all.

### 11.5 Spend cap

The cap is on a month's **usage**, in money, so it reads the same way as the receipt. It's shown on this page as one line ("Spend cap: $20 of usage a month · Change") and set on billing (§14.4). The control is a `ChoiceRow` of amounts (`$20: only what Pro includes`,` $50`, `$100`,`Other…`with a money`TextInput`), then one sentence of what it does: "When this month's usage reaches the cap, new releases and builds wait and what can sleep sleeps. Nothing is deleted, and it resets on the 1st." Pro starts with the cap at $20, what the plan includes, so moving to Pro can't cost more than $20 a month until someone with billing rights raises it.`billing::set_spend_cap`. Hobby has hard caps instead and shows quota bars ("Build minutes 38 of 100", "Database 212 MB of 500 MB").

### 11.6 States, operations, motion

- **States**: loading (skeleton boxes); the estimate failed ("We couldn't check this price. The numbers shown come from the price list loaded at 14:02."); applying (button busy, controls disabled); applied (stamp); failed ("The cluster refused the change: the namespace quota allows 8 CPUs and this needs 12." + **Open events**); a release is rolling out ("Wait for v43 to finish rolling out; scaling during a rollout is refused."); permission denied (developers and viewers see the page read-only: "Scale is changed by owners, admins and developers." for viewers).
- **Operations**: `projects::environments`, `projects::update_environment`, `billing::plans`, `billing::estimate`, `billing::subscription`.
- **Motion**: the selected box's ✓ fades in; receipt lines that changed wash mint (`transition-colors duration-1000`); the slip enters with `Presence::fade().slide_by(0.0, 8.0)`. Reduced motion: the wash shows for a second without fading; the slip appears.
- **Accessibility**: the receipt's total is a `Status` region (`aria_live` polite) that announces "After: $8.24 to $24.24 a month, up $4.00 to $12.00" after the last press, not on every press.

## 12. Environment settings

### 12.1 Variables and secrets: `/<org>/<project>/<env>/variables`

**Purpose**: set what the app reads from its environment; make secrets easy to set and impossible to read back.

```text
Desktop                                                                    Phone: one column, the same rows
Variables and secrets · production                                         stacked: name, then value or
                                                                           version, then the actions
VARIABLES                                               [ + Variable ]
RUST_LOG            info                                Edit   Remove
PUBLIC_SITE_NAME    Notes                               Edit   Remove

SECRETS                                                 [ + Secret ]
SMTP_PASSWORD       ●●●●●●●●  v3 · Ana Lima · 2 Nov     Replace   Remove
STRIPE_SECRET_KEY   ! not set · noticed in .env         Set
DATABASE_URL        set by the cloud · v1               (the cloud manages it)

[ Import a .env file ]   lk cloud secret set NAME          ← CommandBlock
```

- Rows are `fact` rows: the name in mono, the value (variables) or version, setter and date (secrets) in `--cloud-ink-3`, actions as quiet buttons.
- **Secrets are write-only.** **Replace** expands the row into a `PasswordInput` with "New value for SMTP_PASSWORD" and **Save v4**; there is no reveal, because there is nothing to reveal: `config::secrets` returns names and versions only. The section says so once: "Secret values can't be read back through the console, the CLI or the API. To change one, replace it."
- **Names `lk cloud launch` noticed** in `.env` show as `! not set` rows with **Set**; the values never left the computer.
- **Import a .env file** uses `leptoskit_plugin_document_picker` (4 MiB limit, on the web and in the apps). The file is parsed in the client; the page lists each name with "Variable / Secret / Skip" (`ChoiceRow`, defaulting to Secret for names containing `KEY`, `SECRET`, `TOKEN` or `PASSWORD`), then **Set 6 values**. Pasting a whole `.env` would need a multi-line field (§19 gap 4).
- **Taking effect**: a release carries the secret versions it was given (contract §8), so a change applies at the next release. After saving, a mint-wash slip says: "Saved as v4. Production runs with v3 until the next release." with **Ship v42 again with this** (a draft sheet whose fine print lists the changed names). Open question 3 (§22) asks whether a config change should make its own release.
- **Validation**: names are `[A-Z_][A-Z0-9_]*` ("Names use capitals, digits and underscores, and don't start with a digit."); names the cloud manages (`DATABASE_URL`, `PORT`, `LEPTOSKIT_*`) are refused with "The cloud sets DATABASE_URL for this environment."
- **Remove** a secret: `ConfirmPanel`: "Remove STRIPE_SECRET_KEY? Releases after this one won't have it. v42 keeps running with it until the next release." **Remove** / **Keep it**.
- **States**: empty ("No variables yet. Your app reads them with `std::env::var`."), saving, saved (mint wash on the row), failed with the error, offline (actions disabled: "Reconnect to change settings"), read-only for viewers ("Variables are changed by owners, admins and developers. Secret names are shown; values never are.").
- **Operations**: `config::variables`, `set_variable`, `unset_variable`, `secrets`, `set_secret`, `unset_secret`.
- **Back of the card**: the Kubernetes Secret's name, which keys it holds and their versions, and the vault entries; never a value.

### 12.2 Domains: `/<org>/<project>/<env>/domains` and `…/domains/<hostname>`

**Purpose**: put the app on your own name, and know exactly what DNS to set and whether it's right yet.

```text
Desktop list                                                        Detail, desktop and phone (one column)
Domains · production                          [ + Add a domain ]    ┌ notes.example.com ───────────────────┐
notes-ana.leptoskit.app   ✓ Active · always on, can't be removed    │ ! Waiting for DNS                     │
notes.example.com         ! Waiting for DNS · checked 2 min ago  ›  │ Add this record where you manage      │
www.example.com           ✓ Active · certificate renews 2 Feb    ›  │ example.com's DNS:                    │
                                                                    │ ┌ DnsRecords ──────────────────────┐ │
                                                                    │ │ Type   CNAME                     │ │
                                                                    │ │ Name   notes                Copy │ │
                                                                    │ │ Value  edge.leptoskit.app   Copy │ │
                                                                    │ │ ✕ Not found yet                  │ │
                                                                    │ └──────────────────────────────────┘ │
                                                                    │ Last checked 14:20. We check every   │
                                                                    │ few minutes; DNS can take up to a    │
                                                                    │ day to reach everyone.               │
                                                                    │ [ Check now ]      Remove this domain │
                                                                    └──────────────────────────────────────┘
```

- **Add a domain**: one `TextInput` ("notes.example.com"), lower-cased as you type, then **Add**. `domains::attach_domain`, then straight to its detail page.
- **The record table is the page.** `DnsRecords` shows each record the cloud needs (from the API, §20: the contract doesn't return records yet), with Copy on name and value, and per record what we last saw:
  - ✕ "Not found yet";
  - ! "Found, but different: it points to `old-host.example.net`" (we show what we saw, so the person can find the wrong record);
  - ✓ "Matches".
- **Apex domains** (`example.com`) get the A and AAAA records or "If your DNS provider supports ALIAS or ANAME records, point one at `edge.leptoskit.app` instead."
- **States and their words**: `! Waiting for DNS`, `● Checking`, `● Issuing a certificate` ("Usually under a minute"), `✓ Active` ("certificate renews 2 Feb"), `✕ Certificate failed` ("Let's Encrypt couldn't issue one: a CAA record on example.com allows only `digicert.com`. Add `0 issue \"letsencrypt.org\"` to it.") and `• Removed`.
- **Check now**: `domains::verify_domain`; busy for the call, then the table updates, with the changed record washed mint. Disabled for 30 seconds after a check ("Check again in 0:24").
- **Remove this domain**: `ConfirmPanel`: "Remove notes.example.com? It stops serving the app straight away. Your DNS records stay where they are; delete them at your provider." `domains::detach_domain`.
- **Hobby**: **Add a domain** is disabled with "Custom domains are on Pro. Hobby apps use `notes-ana.leptoskit.app`."
- **Back of a domain card**: the hostname object, certificate (issuer, expiry), the route to the app process, and the raw DNS answer last seen.
- **Operations**: `domains::domains` (live, so a domain going active updates on its own), `attach_domain`, `verify_domain`, `detach_domain`.

### 12.3 Database: `/<org>/<project>/<env>/database`

**Purpose**: know what you have, that it's backed up, and be able to go back safely.

```text
Desktop                                                          Phone: the same cards, one column
Database · production
┌ notes ────────────────────────────── 8 cols ┐ ┌ Storage ── 4 cols ┐
│ Postgres 17 · shared · eu-central             │ │ 212 MB of 500 MB  │
│ Migrations: 7 applied, the last 0007_add_tags │ │ ▓▓▓▓▓▓▓▓░░░░░░░░  │
│ by v42 at 14:05                               │ │ Hobby's limit     │
│ Change the size in leptoskit.toml, [cloud.database] │ └─────────────────┘
└───────────────────────────────────────────────┘
┌ Backups · daily, kept 7 days ───────────────────────────────────── 12 cols ┐
│ 18 Nov 03:00   daily           41 MB   Restore…                            │
│ 18 Nov 14:04   before v42      41 MB   Restore…                            │
│ 17 Nov 03:00   daily           40 MB   Restore…                            │
│ [ Back up now ]                                                            │
└────────────────────────────────────────────────────────────────────────────┘
```

- **The size isn't changed here.** `[cloud.database]` in `leptoskit.toml` decides it (contract §5); the card says where, with the TOML for the next size up on its back.
- **Backups**: a list from `databases::database_backups`, newest first, each with its kind ("daily", "manual by Ana Lima", "before v42" when the cloud takes one before a migration; §20 asks whether it does). **Back up now**: `create_backup`, then a `● Backing up` row that becomes the newest entry.
- **Restore** is the most destructive thing in the product, so it gets the most careful panel. **Restore…** expands a `ConfirmPanel` under that row:

  ```text
  ┌ ConfirmPanel, bad wash ─────────────────────────────────────────────┐
  │ Restore production's database to 18 November, 03:00?                │
  │ Everything written since then is replaced: about 11 hours of data.  │
  │ We back up the database as it is now first, so you can come back.   │
  │ notes keeps running; requests that write may fail for the minute    │
  │ the switch takes.                                                    │
  │ Type notes/production to confirm  [                         ]       │
  │ [ Restore to 18 Nov 03:00 ]          Cancel                         │
  └─────────────────────────────────────────────────────────────────────┘
  ```

  The button enables only when the text matches. `restore_backup` with its confirm argument. Then `● Restoring` with the elapsed time, then "Restored to 18 November, 03:00. The backup from before the restore is at the top of the list." The "requests may fail" sentence is the API's to state (§20); until it reports one, the panel says "notes may be unavailable while the switch happens."
- **Continuous backups** (Pro, `backups = "continuous"`): the header reads "Continuous: you can restore to any minute in the last 7 days", but the contract restores named backups only; restoring to a minute is in §20.
- **SQLite apps**: "SQLite on a 1 GiB volume, one copy of notes" with the same backups list (volume snapshots).
- **States**: no database ("notes has no `[db]`, so there's no database."), loading, backing up, restoring, restore failed (what the API said, and "Your data is as it was before you pressed Restore."), permission (restore is owners and admins: "Restoring is for owners and admins. Ask Ana Lima.").
- **Operations**: `databases::database`, `database_backups`, `create_backup`, `restore_backup`.
- **Back of the card**: the CloudNativePG cluster or the shared database and role, the `ScheduledBackup` object, and the WAL archive target.

### 12.4 Project settings: `/<org>/<project>/settings`

- **Environments**: one row each with its pill, what's serving and **Delete…** (not production while it's the only one). Delete is a `ConfirmPanel` typed as `notes/staging`: "Delete staging? Its processes, database, backups, secrets and domains go. Releases stay in the history." `projects::delete_environment`.
- **Delete the project**: at the bottom, separated by a rule: "Delete notes? Every environment, database, backup, secret and domain goes, and its store connections are removed. Releases already in stores stay there." Type `acme/notes`. `projects::delete_project`.
- The project's slug is its URL and the CLI's `[cloud] project`; there's no rename (the contract has none), and the page says "To use another name, create a project with it and ship there."

## 13. Stores

Every other host stops at the server. This is where the cloud is most itself: one sheet that ships a server, an over-the-air update and several store submissions, and an honest split between what the cloud (or your agent) can do and what only a person can.

### 13.1 Stores: `/<org>/<project>/stores`

**Purpose**: see which stores this app reaches, which are ready, and what each is waiting on.

```text
Desktop: a wrapping grid of StoreTiles, 3 per row at xl, 2 at md, 1 on phones
Stores · notes                                                 [ Publish to stores ]▌
┌ App Store ──────────────────┐ ┌ Google Play ────────────────┐ ┌ Solana dApp Store ──────────┐
│ ✓ Connected                 │ │ ✓ Connected                 │ │ ! Needs you                 │
│ 1.4.0 (41)   IN REVIEW 1d   │ │ 1.4.0 (44)   RELEASED       │ │ 1.4.0 (12)   WAITING         │
│ Doctor: 9 ready, 1 warning  │ │ Doctor: 8 ready             │ │ Your wallet's seal          │
│ Check   Details ›           │ │ Check   Details ›           │ │ [ Seal 1.4.0 ]▌             │
└─────────────────────────────┘ └─────────────────────────────┘ └─────────────────────────────┘
┌ Huawei AppGallery ──────────┐
│ • Not connected             │   More stores: Microsoft Store, Samsung Galaxy Store, Meta Horizon,
│ Reaches HarmonyOS and       │   itch.io, Steam, Amazon Appstore, Snap Store ›
│ Android in China.           │
│ [ Connect ]                 │
└─────────────────────────────┘
```

- Tiles are shown for the stores `[publish]` names and the platforms `[app] platforms` builds; the rest are one line of links. Each `StoreTile` is a `TurnCard`: its back lists the credential names in the vault, their versions, who set them and when, and the call the last check made ("`GET /v1/apps?filter[bundleId]=com.acme.notes` → 200").
- The tile's stamp is the latest store release's (`IN REVIEW`, `RELEASED`, `WAITING`, `FAILED`), with its age.
- **Operations**: `stores::store_connections`, `store_releases` (live).
- **Empty** (no `[publish]` and no store platforms): "notes builds for the web only, so there's nothing to send to a store. Add `ios` or `android` to `[app] platforms` to build apps." with a link to the docs' store guide.

### 13.2 Connect and check a store: `/<org>/<project>/stores/<store>`

**Purpose**: give the cloud the credentials a store needs, once, and find out before a release what would stop it.

```text
Desktop: credentials (7 columns) beside the doctor (5); phones: credentials, then the doctor
App Store
┌ Credentials ─────────────────────────────────────┐ ┌ Doctor ─────────────────────────────┐
│ App Store Connect API key                        │ │ ✓ The key signs in                  │
│ Issuer ID  [ 69a6de70-…               ]          │ │ ✓ com.acme.notes exists             │
│ Key ID     [ 2X9R4HXF34               ]          │ │ ✓ The version 1.4.0 is new          │
│ Key file   AuthKey_2X9R4HXF34.p8 · v2 · Ana Lima │ │ ! No privacy policy URL             │
│            [ Choose a .p8 file ]                 │ │   Needed before review. Only you    │
│ Where to get these: App Store Connect → Users    │ │   can add it in App Store Connect.  │
│ and Access → Integrations ↗                      │ │ ✓ Build number 41 is higher than 40 │
│ [ Save and check ]▌          Disconnect          │ │ Checked 14:20 · [ Check again ]     │
└──────────────────────────────────────────────────┘ └─────────────────────────────────────┘
```

- **Fields come from the store client's `NEEDED` list**, the same one `lk publish` uses, so the cloud and the CLI ask for the same things in the same words. Text values use `TextInput`; key files (`.p8`, a Play service-account `.json`, a keystore) use `leptoskit_plugin_document_picker`, read in the client and sent once. Like secrets, they're write-only: the page shows the file name, version, who set it and when, and offers **Replace**, never the contents.
- **The Solana dApp Store asks for no private key.** Its credentials are the publisher wallet's address and the app's collection address, with this under them: "We never hold your wallet. Each release is sealed by your wallet when it's ready (§13.6)." The doctor checks the publisher's balance, that the APK's signing key differs from Google Play's, and that `versionCode` is one higher than the last release.
- **Doctor**: `DoctorList`, one row per check with ✓ ready, ! warning (ships, but fix it), ✕ blocks (a release would fail here), or ● checking. Each row's sentence says what was checked and, when it isn't ready, who fixes it: "Only you can add it in App Store Connect" (a person step) or "Run `lk icons` and ship again" (an agent can). The contract has no doctor operation; §20 asks for `check_store`, which runs the store client's dry-run checks on a worker.
- **States**: not connected (the form, empty), saving, checking, connected and ready, connected with warnings, connected but blocking, credential rejected ("Apple refused this key: it was revoked or the Key ID doesn't match the file."), disconnected.
- **Disconnect**: `ConfirmPanel`: "Disconnect the App Store? We delete these credentials from the vault. Builds already in App Store Connect stay there."
- **Operations**: `stores::connect_store`, `disconnect_store`, `store_connections`; `check_store` (§20).

### 13.3 One release to several stores: `/<org>/<project>/publish`

Publishing is a release sheet (§9) whose lines are stores. It has the same anatomy, signature and stamps, so there's nothing new to learn:

```text
┌ sheet ───────────────────────────────────────────────────────────── [Workings] ┐
│ STORE RELEASE                                                                  │
│ 1.4.0                                                     notes · from v42     │
│ ────────────────────────────────────────────────────────────────────────────── │
│ [✓] App Store       iOS build 41 · TestFlight, then review     ≈ 18 macOS min  │
│ [✓] Google Play     Android build 44 · internal track          ≈ 9 Linux min   │
│ [✓] Solana dApp Store  Android build 12 · needs your wallet's seal             │
│ [ ] Huawei AppGallery  not connected · Connect ›                               │
│ ────────────────────────────────────────────────────────────────────────────── │
│ Submit for review when uploaded   [ on ]                                       │
│ Builds: about $0.77 (18 macOS minutes, 9 Linux minutes)                        │
│ ────────────────────────────────────────────────────────────────────────────── │
│ WHAT THIS RELEASE WON'T DO                                                     │
│ • Reviews take as long as each store takes: usually a day for Apple and Play,  │
│   1–2 days for a dApp Store update. We can't speed them up.                    │
│ • Store builds can't be recalled from here once a store has released them.     │
│ ────────────────────────────────────────────────────────────────────────────── │
│ Sign here ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄                 [ Sign and publish 1.4.0 ]▌   │
└────────────────────────────────────────────────────────────────────────────────┘
```

- Each store line has a `Toggle` while the sheet is a draft (a `Switch` labelled "Include the App Store"); a store whose doctor blocks shows ✕ and its blocking check instead, and can't be included.
- After signing (`stores::publish` with the chosen stores and `submit`), the stamps per store line are `BUILT` → `UPLOADED` → `IN REVIEW` → `RELEASED`, or `WAITING` when a person step or the seal is outstanding, or `FAILED` with the store's reason ("Apple rejected the build: ITMS-90683, `NSCameraUsageDescription` missing from Info.plist").
- A server release with `[publish]` set has the same store lines under its server lines, so a single `lk deploy --publish` makes one sheet with everything.
- **Operations**: `stores::publish`, `store_releases` (live), `store_release`; `billing::estimate` for build minutes.

### 13.4 What to do next: agent steps and person steps

Every store release ends with the store client's report (`publish::steps::Report`). The cloud draws it as two lists, under the sheet, because the difference between them is the point:

```text
Desktop: two columns; phones: "Only you" first, then "Your agent can"
┌ Only you can do these (2) ─────────────────────────┐ ┌ Your agent can do these (3) ──────────────┐
│ ○ Add a privacy policy URL                         │ │ ○ Add NSCameraUsageDescription            │
│   App Store · Apple asks the account holder.       │ │   to the iOS Info.plist                   │
│   Open App Store Connect → notes → App Privacy ↗   │ │   Why: the build uses the camera plugin.  │
│   Enter  Privacy Policy URL                        │ │   Done when: the next build uploads.      │
│          https://notes.example.com/privacy   Copy  │ │ ○ Bump versionCode to 13 for the dApp     │
│   Why: review can't start without it.              │ │   Store                                   │
│   Done when: App Store Connect shows it saved.     │ │ ○ Run lk icons for the Play feature       │
│   [ Mark done ]   Not now                          │ │   graphic                                 │
│ ○ Seal 1.4.0 with your wallet       [ Seal ]▌      │ │ [ Copy all for your agent ]               │
└────────────────────────────────────────────────────┘ └───────────────────────────────────────────┘
2 of 7 done
```

- **Person steps** (`Who::Person`) show the store, what, why, the URL as a link that opens the store's console, each field to enter with **Copy**, and "Done when". They also go to **To do** (§13.5) and, when the person allows notifications, as a push: "notes 1.4.0 needs you: add a privacy policy URL in App Store Connect."
- **Agent steps** (`Who::Agent`) show the same fields, plus **Copy all for your agent**, which copies one Markdown block: the release, each step with its file, command and "done when", and the command that checks them (`lk publish --check`). The agent fixes the repository; the next release clears them.
- **Done**: detected steps tick themselves when the next check passes ("✓ Detected at 14:32"); person steps the cloud can't see can be marked done ("✓ Marked done by Ana Lima"), and are re-checked by the next release. The contract's report has no done state per step; §20 asks for it.
- **Empty**: "Nothing to do. Each store has what it needs."
- Components: `StepList`, `CommandBlock`, `OnceValue` isn't used here (nothing is secret).

### 13.5 To do: `/todo`

**Purpose**: one inbox for everything waiting on a person, across organisations, in the order it blocks things.

- Order: seals (they expire), person steps for releases in flight, a reached cap, a failed payment, invitations, then suggestions ("Add a passkey for one-touch sign-in").
- Each item is one row: what, where ("acme / notes · 1.4.0"), how long it's waited, one action. A seal's row has **Seal** as its primary button.
- The tab bar's **To do** and the top bar's count are the same number. Push notifications use `leptoskit_notifications` with a topic per person.
- **Empty**: "Nothing needs you." in the hand face, the one time an empty state is handwritten, because it's the product telling you you're done.
- **Operations**: an aggregate query (§20: `todo`), built from `store_releases`, `signing_request`, `subscription` and invitations.

### 13.6 The seal: `/<org>/<project>/sign/<request>`

**Purpose**: have the publisher's wallet sign a Solana dApp Store release, which the store requires, without the cloud ever holding the wallet.

This is the product's one cryptographic signature, and the design treats it as the moment the sheet metaphor was built for. The sheet already has a signature line; this adds a **seal**: a round ember stamp, the size of a coin, that only your wallet can press.

```text
Desktop: the sheet centred, max-w-xl; phones: full width
┌ sheet ───────────────────────────────────────────────────────────┐
│ SEAL A RELEASE                                                   │
│ notes 1.4.0 for the Solana dApp Store                            │
│ The dApp Store asks your publisher wallet to sign each release.  │
│ We've prepared what it needs. Your wallet signs; we never see    │
│ your key.                                                        │
│ ──────────────────────────────────────────────────────────────── │
│ 1  Mint the release NFT          a transaction · ≈ 0.015 SOL rent │
│ 2  Add it to notes' collection   a transaction · network fee      │
│ 3  Attest the release            a message · free                 │
│ ──────────────────────────────────────────────────────────────── │
│ Publisher wallet  7xKX…9fQ2                       Copy            │
│ Balance           0.42 SOL ✓ enough                               │
│ ──────────────────────────────────────────────────────────────── │
│                         ╭──────╮                                  │
│                         │ SEAL │   ← Seal, unpressed: ember outline│
│                         ╰──────╯                                  │
│ [ Sign with Phantom ]▌   [ Sign with Solflare ]                   │
│ Sign on your phone instead  ▦ QR                                  │
└──────────────────────────────────────────────────────────────────┘
```

**Where it can be signed**:

- **In a browser**: the wallets `wallet_apps()` finds, one button each, named for the wallet. Signing the two transactions needs Wallet Standard's `signTransaction`, which leptoskit's wallet support doesn't have (it signs sign-in messages only): §19 gap 6.
- **In the cloud app on a Solana phone**: Mobile Wallet Adapter to the Seed Vault, so the seal is pressed with the phone's fingerprint or PIN. Native wallets answer `unsupported` today; the same gap 6.
- **From another device**: **Sign on your phone instead** shows a `QrCode` (with the URL as text and Copy) that opens this page in the cloud app.
- Without a wallet anywhere: "No wallet found here. Install a Solana wallet with Wallet Standard support, or sign on your phone." Never a private-key field.

**Signing**: pressing **Sign with Phantom** asks the API to prepare the first transaction now (`stores::signing_request` with fresh blockhashes, §20: prepared lazily, because a blockhash expires in about a minute), hands it to the wallet, and sends the result with `submit_signature`. The `Seal` shows "Signing 1 of 3"; each item ticks as its signature is accepted. When the third is in, the seal **presses**: the ember fills, `SEALED` with the time appears in `--cloud-on-ember`, and on phones a light haptic tap (`leptoskit_plugin_haptics`). The same seal then appears, small, on the release sheet's dApp Store line, and the store release moves to `IN REVIEW`: "Reviews take 1–2 days for an update and 3–4 for a new app."

**States and copy**:

| State               | Seal               | Copy                                                                                                                                                 |
| ------------------- | ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| waiting             | ember outline      | (as above)                                                                                                                                           |
| preparing           | outline, busy      | "Preparing the transaction…"                                                                                                                         |
| signing { n of m }  | outline, `n` ticks | "Approve it in Phantom. Signing 1 of 3."                                                                                                             |
| sent, unconfirmed   | outline, `UNKNOWN` | "Sent. Waiting for the network to confirm it (0:12). If it isn't confirmed in a minute, we check whether it landed before asking you to sign again." |
| declined            | outline            | "Phantom declined. Nothing was sent. Sign again when you're ready."                                                                                  |
| expired (blockhash) | outline            | "That took longer than the network allows, so we've prepared it again. Approve the new one."                                                         |
| wrong wallet        | outline, ✕         | "Phantom is signed in as 4mPq…a81c. This release needs the publisher wallet 7xKX…9fQ2. Switch accounts in Phantom."                                  |
| low balance         | outline, !         | "Minting needs about 0.016 SOL and 7xKX…9fQ2 has 0.004 SOL. Add SOL to it, then press Check again."                                                  |
| request expired     | outline, `EXPIRED` | "This request expired after 24 hours. [ Prepare it again ]"                                                                                          |
| sealed              | filled, `SEALED`   | "Sealed at 14:41. The dApp Store has the release; review usually takes 1–2 days."                                                                    |

- **Operations**: `stores::signing_request`, `submit_signature`, `store_release`; `leptoskit_auth::client::wallet_apps`.
- **Motion**: the press is `entering=Presence::fade().scale(1.25).spring(Spring::SNAPPY)` on the filled layer over the outline; the ticks fade in. Reduced motion: the seal fills at once.
- **Accessibility**: the seal is a `Status` region: "Seal: 2 of 3 signed", then "Sealed at 14:41". Each item is a list item with its state in words. The wallet address is read in groups ("7 x K X, ending 9 f Q 2").
- **Back of the sheet**: each prepared transaction decoded (program, accounts, the rent in lamports), the attestation message as text, and the signatures once made, so a careful publisher can check exactly what they're signing.

## 14. Organisation and account

Hidden until it matters (§2): one person in a personal organisation sees **Billing** and **Tokens** under **You**, and no members page until they invite someone.

### 14.1 Organisations: `/organizations`

A list of organisations with your role in each and their plan; **Create an organisation** expands an inline form (name; the slug follows it, editable, checked as you type). `account::organizations`, `create_organization`. Empty never happens: everyone has their personal organisation.

### 14.2 Members and invitations: `/<org>/settings/members`, `/invitations/<token>`

```text
Members · acme                                                   [ + Invite ]
Ana Lima      ana@example.com   owner       last active today      (you)
Ben Ode       ben@acme.dev      billing     last active 3 days     Change role   Remove
cara@acme.dev                   developer   invited · expires in 6 days        Revoke

What each role can do                                              (a TurnCard's back)
owner      everything, including deleting the organisation
admin      everything except billing and deleting the organisation
developer  ship, roll back, change settings, connect stores
billing    plan, invoices, the spend cap
viewer     see everything except secret values
```

- **Invite**: email and a role (`ChoiceRow` with the line above under each role), **Send invitation**. `account::invite_member`. Pro only; Hobby says "Teams are on Pro."
- **Change role**: the row expands into the role `ChoiceRow` and **Save**. `set_member_role`. The last owner can't be demoted or removed: "acme needs at least one owner."
- **Remove**: `ConfirmPanel`: "Remove Ben Ode from acme? Their tokens for acme stop working now." `remove_member`.
- **The invitation page** is a sheet with a signature line, like sign-up: "Ben Ode invited you to acme as a developer." **Join acme** (`accept_invitation`). Signed out, it goes through `/sign-in?next=…`. Signed in as someone else: "This invitation is for cara@acme.dev, and you're signed in as ana@example.com. Sign out and use that address, or ask Ben for a new invitation." Expired: "This invitation expired on 12 November. Ask Ben Ode for a new one."

### 14.3 API tokens: `/<org>/settings/tokens`

- **List**: name, scopes as words, the project it's limited to, who created it, last used, expiry. **Revoke** (`ConfirmPanel`: "Revoke ci-deploy? Anything using it stops working now.").
- **Create**: name; scopes as `Toggle`s (`read`, `deploy`, `manage`, `publish`, `billing`, `admin`, each with its one-line meaning); the project (`ChoiceRow`: all projects or one); expiry (`30 days`, `90 days`, `a year`; there's no "never", because the contract expires tokens by default). **Create token**.
- **Shown once**: an `OnceValue` card on the underside (ink, mono, so it reads as machinery): the token, **Copy**, and "Copy it now. You won't see it again; we keep only a hash." with how to use it: `LEPTOSKIT_CLOUD_TOKEN` in a `CommandBlock`. **I've saved it** removes it for good; leaving the page does too, after one line of warning that the token stays valid.
- `tokens::tokens`, `create_token`, `revoke_token`.

### 14.4 Billing: `/<org>/settings/billing`

**Purpose**: what you're paying, what it's for, and the cap, with nothing alarming unless it should be.

```text
Desktop: the plan and cap (5 columns) beside the receipt (7)
┌ Plan ─────────────────────────────────┐ ┌ November so far ── sheet ───────────────────────┐
│ Pro · $20 a month                     │ │ USAGE              QTY            RATE    AMOUNT │
│ $20 of usage included                 │ │ Compute, s         1,140 h        $4.00/mo $6.25 │
│ Change plan   Payment and invoices ↗  │ │ Compute, m         21 h           $8.00/mo $0.23 │
└───────────────────────────────────────┘ │ Database, shared   212 MB         included $0.00 │
┌ Spend cap ────────────────────────────┐ │ Builds, Linux      240 min        $0.005   $1.20 │
│ $50 of usage a month         Change   │ │ Builds, macOS      36 min         $0.04    $1.44 │
│ ┌ chart ────────────────────────────┐ │ │ Egress             12 of 100 GB   included $0.00 │
│ │ cap ────────────────────────  $50 │ │ │ ──────────────────────────────────────────────── │
│ │ included ···················  $20 │ │ │ Usage                                      $9.12 │
│ │              ___---- - - - -  $14 │ │ │ Included in Pro                           −$9.12 │
│ │  ____-----‾‾‾                     │ │ │ Plan                                      $20.00 │
│ └───────────────────────────────────┘ │ │ So far                                    $20.00 │
│ Usage forecast $14 by 30 Nov · fits ✓ │ │ Updated 3 min ago. Usage forecast $14, inside   │
└───────────────────────────────────────┘ │ the $20 included, so you'll pay $20.00.         │
                                          └──────────────────────────────────────────────────┘
INVOICES
Oct 2026   $20.00   Paid    PDF ↗
Sep 2026   $23.40   Paid    PDF ↗
```

- **The receipt** is the "usage by meter" view (§2): `Receipt` on sheet paper, one line per meter with quantity, rate and amount, then the included credit, plan and total. The total says when it was last updated (usage is sampled). Each line's back is its arithmetic and the samples behind it.
- **One chart**: spend so far this month as a line, the forecast as a dashed continuation, the included $20 and the cap as horizontal rules with their amounts, the admin's `line_chart` (`Vector` paths). Its `accessibility_label` is the sentence under it: "Forecast $14 by 30 November; that fits your $50 cap."
- **Plan**: Hobby, Pro or Scale with its price and what's included. **Change plan** goes to `billing::checkout` (Stripe Checkout) on the web; **Payment and invoices ↗** goes to `billing::portal`. In the iOS and Android apps, the page shows the plan, usage and cap and sends plan changes to the web ("Change your plan at cloud.leptoskit.dev"), which keeps the apps clear of store purchase rules.
- **Invoices**: the contract has none; until §20's `invoices` lands, the list is replaced by **Payment and invoices ↗**.
- **States**:
  - Hobby: quota bars instead of the receipt ("Build minutes 38 of 100 · resets 1 December", "Database 212 MB of 500 MB"), and **Move to Pro** with Pro's three most useful differences;
  - forecast over cap: the forecast line and the cap word are warn: "! At this rate you'll reach your $50 cap around 26 November. Then new releases wait and what can sleep sleeps.";
  - cap reached: bad, with what stopped and what didn't;
  - payment failed: bad panel at the top: "The payment on 1 November didn't go through. Releases keep working until 15 November. Update the payment method ↗";
  - stale: "Usage last updated 2 hours ago; samples are delayed." (warn word, not a number in red);
  - permission: developers and viewers see the plan and the cap, not invoices or the receipt ("Billing is for owners and billing members.").
- **Operations**: `billing::plans`, `subscription`, `usage`, `checkout`, `portal`, `set_spend_cap`; `invoices` and a usage forecast with `updated_at` in §20.

### 14.5 Account: `/account`

- **You**: name and email.
- **Sign-in methods**: GitHub, Google and email, each linked or **Link**; passkeys as a list (device name, added, last used, **Remove**) with **Add a passkey** (`add_passkey`); Solana wallets linked (`link_wallet`), shown by short address.
- **Signed in on**: sessions and CLI devices from `devices` ("ana-mbp · lk cloud 0.9.0 · used 2 minutes ago", **Sign out**), so a lost laptop's CLI can be cut off here.
- **Appearance**: System, Light, Dark (`ChoiceRow`).
- **Sign out** at the bottom.

## 15. Operator views: `/operator…`

For the few people who run a cloud. Plain tables, the same components, status words, no stamps, no sheet. Only the operator role sees **Operator** in the rail.

- **Clusters** (`/operator`, `/operator/clusters/<id>`): name, region, provider, nodes ready of total, the WireGuard overlay (peers connected of expected, per node, from the contract's node network), the cluster worker's last heartbeat, environments running, and **Reconcile** (`operator::reconcile`). A cluster detail lists its nodes with their provider, overlay address and last handshake. **Register a cluster** (`register_cluster`) is a short form that ends in a pairing code.
- **Workers** (`/operator/workers`): role (build or cluster), pool (shared or an organisation), region, capabilities, version, last claim, the current job and its lease's expiry. **Revoke** (`revoke_worker`, `ConfirmPanel`: "Revoke builder-eu-3? Its key stops working now; its current job is reconciled, not repeated.").
- **Pairing** (`/operator/pairing`): **Issue a pairing code** for a role and pool → `OnceValue` with a ten-minute countdown ("Expires in 9:41") and the command `leptoskit-cloud-worker pair --code …` in a `CommandBlock`.
- **Unknown outcomes**: jobs whose worker died mid-effect, each with what the cluster reports now and **Reconcile**. This is where a release sheet's `UNKNOWN` (§9.4) gets resolved, and the sheet updates when it is.
- `operator::clusters`, `register_cluster`, `workers`, `issue_pairing`, `revoke_worker`, `reconcile`.

## 16. States across the product

Every screen above lists its own states. These are the rules they share, with the copy pattern and the component.

| State                   | What it looks like                                                                                                      | Copy (pattern, then an example)                                                                                                   |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Loading                 | skeletons in the final layout (`bg-[var(--cloud-sunken)] rounded-lg`), no shimmer; a busy region has `aria_busy(true)`  | none on screen; the region's label is "Loading releases"                                                                          |
| Empty                   | `empty_state`: one sentence of what goes here, one action, the CLI command as a `CommandBlock` when there is one        | "No domains yet. Add one, or run `lk cloud domain add notes.example.com`."                                                        |
| Error loading           | the region keeps its frame; ✕, what failed, the error's code, **Try again**                                             | "We couldn't load the releases: `rpc: unavailable`. Try again."                                                                   |
| Error acting            | inline under the control that acted, ✕ and the API's sentence; the control is enabled again                             | "The domain wasn't added: notes.example.com is already attached to acme/blog."                                                    |
| Offline                 | `OfflineChip` in the top bar; values keep their last state with "as of" times; every mutation is disabled with a reason | "Offline · showing 14:02" / button description "Reconnect to ship"                                                                |
| Reconnecting            | the chip, warn                                                                                                          | "Reconnecting · trying again in 8s" (from `use_live_status()`'s `next_in`)                                                        |
| Signed out              | the page stays; a sheet-styled panel replaces the content                                                               | "You've been signed out. Sign in and we'll bring you back here."                                                                  |
| Permission denied       | the page renders read-only where viewing is allowed; actions are disabled with who can act                              | "Restoring is for owners and admins. Ask Ana Lima or Ben Ode."                                                                    |
| Not found, or no access | one response for both, so a URL doesn't reveal what exists                                                              | "There's no project at acme/notes, or you can't see it. Your organisations →"                                                     |
| Sleeping app            | info word, never a problem colour; the wake time from the last wake                                                     | "Asleep. The next request wakes it; that took 1.2s last time."                                                                    |
| Cold start              | the topology node's `waking` state; the URL row adds nothing alarming                                                   | "Starting for a request."                                                                                                         |
| Cap reached             | one warn panel per page that it affects, with what stopped and what didn't                                              | "You've reached this month's $20 spend cap. New releases wait. Environments that can sleep are asleep. Nothing has been deleted." |
| Person-only step        | a ○ row in "Only you can do these", also in To do                                                                       | "Only you can do this: Apple asks the account holder."                                                                            |
| Unknown outcome         | the `UNKNOWN` stamp; no failure colour; no actions that assume either outcome                                           | "We don't know yet whether v42 shipped. We're checking what the cluster is running."                                              |
| Stale data              | the "updated" time turns warn after 30 minutes                                                                          | "Usage last updated 2 hours ago; samples are delayed."                                                                            |
| A newer console         | the cloud apps update themselves over the air like any leptoskit app; the web reloads on navigation                     | "A newer version of the console is ready. It loads the next time you open the app."                                               |

The rules: an error names what failed and what still works; a state that isn't a problem never uses a problem colour; nothing is ever shown as failed until the API says so.

## 17. The public site: how it works, and pricing

The landing page's cloud section (`crates/leptoskit_app/src/landing.rs`, `fn cloud`) describes the hosted pilot today. It becomes the product's front door. It stays in the site's register, Skribble ink (`--sk-*` tokens, sketch lettering, rough frames), because it's the public site, and it uses the site's `intro` and `rise`/`arrive` motion. The cloud's ideas show up in that register: the sheet, the signature and the stamps.

### 17.1 How it works

```text
Desktop: intro, then the sheet (7 columns) beside three steps (5)        Phone: intro, steps, then the sheet

LEPTOSKIT CLOUD                                    ← eyebrow
Sign it. We'll ship it everywhere.                 ← sketch display
Your server, database, web app, over-the-air updates and store releases, from one file and one command.

┌ a sheet in the site's rough frame ───────────────┐   1  $ lk cloud login
│ RELEASE v1                notes · production     │      Sign in once, from the terminal.
│ Server            1 × xs        $2.00   BUILT    │
│ Database          shared        free    MIGRATED │   2  $ lk cloud launch
│ Web               edge          free    LIVE     │      It reads leptoskit.toml, shows what it
│ iOS, Android      over the air  free    LIVE     │      will ship and what it costs, and asks.
│ App Store, Play   1.0.0         —       IN REVIEW│
│ ──────────────────────────────────────────────── │   3  Every release is a signed sheet
│ Signed off by  you                       ╱LIVE╱  │      Each place it ships stamps it. Roll back
└──────────────────────────────────────────────────┘      by signing an old one again.
```

- The sheet is static data, rendered with the cloud's own `ReleaseSheet` in the site's colours. As it scrolls into view (`in_view`), its lines write in, the signature writes, and the stamps land one by one, the same motions as §4. Reduced motion: everything at rest.
- Under it, one line in the site's hand: "Show your workings: turn any card over to see the Kubernetes underneath." with a small sheet that turns over when pressed (the one interactive piece; a `TurnCard` with the site's colours).
- Copy names nothing that hasn't been done (contract §12): no uptime numbers, no customer logos, no "production-ready" until a cluster is running and someone has been billed. Until then the eyebrow reads **LEPTOSKIT CLOUD · EARLY ACCESS** (open question 5).

### 17.2 Pricing

Pricing is a receipt, because that's how the console shows money, and because a feature grid with ticks is the generic thing to avoid:

```text
Desktop: three narrow receipts side by side; phones: stacked, Pro first

┌ HOBBY ─────────────────┐ ┌ PRO ───────────────────────┐ ┌ SCALE ─────────────────┐
│ Plan            $0     │ │ Plan                 $20   │ │ By agreement           │
│ One project, size xs,  │ │ Usage included      −$20   │ │ Dedicated nodes, or    │
│ sleeps when idle       │ │ Teams, domains, any size,  │ │ your own cluster with  │
│ Shared database 500 MB │ │ always on, autoscaling,    │ │ our workers.           │
│ 100 build minutes      │ │ continuous backups, stores │ │                        │
│ ────────────────────── │ │ ────────────────────────── │ │ ────────────────────── │
│ A month         $0     │ │ A month       from $20     │ │ Talk to us             │
│ Can't charge you.      │ │ Spend cap starts at $20.   │ │                        │
│ [ Start free ]         │ │ [ Start on Pro ]           │ │ [ Get in touch ]       │
└────────────────────────┘ └────────────────────────────┘ └────────────────────────┘

WHAT THINGS COST ON PRO, BY THE SECOND
Size xs  $2 a month always on · s $4 · m $8 · l $16 · xl $32
Database s (1 GiB) $5 · m (2 GiB) $10 · l (4 GiB) $20 · storage $0.20 a GB-month
Builds: Linux $0.005 a minute · macOS $0.04 a minute

FOR EXAMPLE
An always-on m app             $8.00
with an s database             $5.00
                              ──────
                              $13.00, inside Pro's included $20
```

- The prices are the catalogue's (contract §11); this page reads them from the same data the console does, so the two never disagree.
- Under the receipts, three questions in the voice of §6, answered in one or two sentences each:
  - "What happens at my cap?" New releases wait and what can sleep sleeps; nothing is deleted.
  - "What does sleeping cost me?" Nothing while asleep, and the first request waits about a second while it starts.
  - "Can I leave?" Your app is a leptoskit app: `lk deploy` runs it on your own infrastructure with `[infra]`.
- **Start free** and **Start on Pro** go to `/sign-in` (Pro with `?next=/<org>/settings/billing`).

## 18. Accessibility

What every screen above follows, gathered in one place:

- **Contrast**: every text token is ≥ 4.5:1 on the surfaces it's used on, and every control boundary ≥ 3:1 (§4 lists the numbers). The two admin colours that fail are replaced (§4, "Changes from the admin").
- **Never colour alone**: every state is an icon and a word; stamps are words; selection is a ✓ and a thicker border; changes are a mint wash and, for screen readers, the new value in a live region.
- **Targets**: every pressable is at least 44 × 44 (`min-h-11 min-w-11`), including mint tabs, Copy buttons and stepper buttons.
- **Focus**: the ember ring on every focusable element (§4); focus order follows reading order, which each screen states where it isn't obvious; expanding a `ConfirmPanel` or a row moves focus into it and closing returns it to the control that opened it. There are no overlays, so there are no focus traps to escape (§19 gap 9).
- **Keyboard**: everything works with Tab, Enter and Space; `ChoiceRow` and `Stepper` take the arrow keys; `/`, `W`, `Shift+W` and `Esc` are shortcuts, never the only way.
- **Screen readers**: stamps, the stage rail, the seal and the topology have sentence labels (§9.10, §10.2, §13.6); one polite live region per page announces stage changes and finished actions, and failures are assertive. In the generated iOS and Android apps live regions don't announce yet (§19 gap 3).
- **Reduced motion**: every motion in §4 has its at-rest form. `Motion` honours the setting on the web today; transitions and presences need gap 2.
- **Time**: nothing moves on by itself on a timer (§7.4), countdowns are text that's read, and the seal's one-minute blockhash limit is handled by preparing again, never by rushing the person.
- **Text size**: text is set in pixels; following the system text size in the native apps is part of gap 1.

## 19. Gaps in leptoskit

Each gap is something this design needs that leptoskit doesn't do yet, what the design does until it does, and the feature it asks for. None is designed around silently.

| #  | Gap                                                                                               | Where it shows                                                                                      | Today                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Proposed feature                                                                                                                                                                                                                                              |
| -- | ------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1  | Text on native: font families, `tabular-nums`, `truncate`, `tracking-*`, and the system text size | signatures and first-run headings (hand face), stamps (mono), price columns, the system text size   | web only; the iOS and Android apps draw the system face at fixed sizes, so signatures lose the hand and columns of prices don't align                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | bundle fonts as app assets and map `font-[…]` to `UIFont`/`Typeface`; map `tabular-nums` to the platforms' monospaced digits; `truncate` and `tracking-*` to label properties; scale text with Dynamic Type and Android's font scale                          |
| 2  | Reduced motion for transitions and presences, and a way to read it                                | turn over, stamps, the seal, the column, the write-in on native                                     | `Motion` honours `prefers-reduced-motion` on the web; transitions and `entering` presences don't, anywhere; only Rive reads the setting natively                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | a `reduced_motion()` signal on every backend, and transitions and presences that skip to their end state when it's on                                                                                                                                         |
| 3  | Live regions in the generated iOS and Android apps                                                | stage changes, "copied", the seal's progress, the receipt's total                                   | `aria_live` and `Role::Status` reach the web; the generated apps' renderers don't map them, so nothing is announced                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | map polite and assertive live regions to `UIAccessibility.post(.announcement)` and `accessibilityLiveRegion` in `leptoskit_bridge_ffi/mobile`                                                                                                                 |
| 4  | Multi-line text input on native                                                                   | importing a pasted `.env`, pasting a PEM key, a note when marking a step done                       | `aria_multiline` is a label; native text fields stay one line                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | `TextInput().multiline(rows)` drawn as `UITextView`/multi-line `EditText` and a `<textarea>` on the web                                                                                                                                                       |
| 5  | Breaking long words                                                                               | logs, digests, URLs, DNS values                                                                     | the Tailwind compiler has no `break-all` or `break-words` on any backend, so on the web too a long token overflows; UI 1 lets them run on                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | `break-all` and `break-words` mapped to character wrapping on UIKit, AppKit and Android                                                                                                                                                                       |
| 6  | Solana transaction signing                                                                        | the seal (§13.6)                                                                                    | `leptoskit_auth` signs sign-in messages through Wallet Standard in the browser; native wallets answer `unsupported`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | `sign_transactions(&[bytes])` and `sign_message(bytes)` through Wallet Standard on the web and Mobile Wallet Adapter (Seed Vault) on Android; iOS through wallet deep links or not at all, stated                                                             |
| 7  | Device-flow request origin                                                                        | `/device`: "Started 40 seconds ago, near Berlin" (§7.4)                                             | `DeviceRequest` carries the app, scopes and code, not when or where the request started                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | add `requested_at`, a coarse location from the requester's IP, and the client's host name to `leptoskit_idp`'s device request                                                                                                                                 |
| 8  | Dashed and dotted borders                                                                         | the unsigned signature line, asleep topology nodes                                                  | the compiler has no `border-dashed`; the design draws them as `Vector` paths with `Stroke::dashes`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | `border-dashed` and `border-dotted` on every backend                                                                                                                                                                                                          |
| 9  | Focus scopes and overlays                                                                         | why there's no command palette, no dropdowns, no modal confirms and no bottom sheets (§2, §3)       | elements can be focused, but there's no focus scope, trap or restore, and no overlay layer                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | an `Overlay` with a focus scope, `Esc`, focus restore and `aria_modal` on every backend; then menus and dialogs built on it                                                                                                                                   |
| 10 | Route paths for axum 0.8                                                                          | the console's server                                                                                | `leptoskit_router`'s `axum_path` writes `:org`, which `leptos_axum` 0.9 refuses at startup, and the not-found catch-all can't sit beside `/[org]` in axum's table; the console rewrites `:name` to `{name}` and renders any address no page claims from a fallback, which answers with the not-found page and a 404                                                                                                                                                                                                                                                                                                                                                                                                | `axum_path` writes `{name}`, and the generated route list leaves the catch-all to a fallback                                                                                                                                                                  |
| 11 | A focus variant                                                                                   | the ember focus ring on every control (§4, §18)                                                     | the compiler has no `focus:` or `focus-visible:` variant; each control sits in a `focus_ring` that draws the ring from a signal the control's focus and blur set, so it shows for pointer focus too, not only the keyboard                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | `focus-visible:` on every backend                                                                                                                                                                                                                             |
| 12 | Keys with nothing focused                                                                         | `/`, `W`, `Shift+W`, `Esc` (§3)                                                                     | `on_key` on the root view hears keys only while focus is inside it, so straight after a page loads the keys do nothing until something is focused; nothing more is done, since the keys are shortcuts, never the only way                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | a window-level key handler, or focus placed on the page's main region when it loads                                                                                                                                                                           |
| 13 | A page's document title                                                                           | every page                                                                                          | there's no way for a leptoskit page to set `<title>`, so every tab reads "LeptosKit Cloud"; each page names itself in its `h1`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | a title on file-route pages, set on the web and as the window title on desktop                                                                                                                                                                                |
| 14 | A remembered appearance                                                                           | `/account`, Appearance                                                                              | `leptoskit::appearance::set` lasts until the page reloads; the card says so                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | an appearance that persists per device on every backend                                                                                                                                                                                                       |
| 15 | **Blocking.** Semantic elements kept across renders on the web                                    | every page in a browser: typing, pressing, focus                                                    | `leptoskit_native` draws `main`, `nav`, `ul`, `li`, `section`, `h1`–`h6` and the other landmark tags with tachys' `custom("main")`. tachys 0.3.0-rc1 rebuilds a custom element in place only when its tag equals the DOM's `tagName`, which the browser upper-cases (`MAIN`), so every render replaces each of those elements and everything inside it. In the console, whose pages sit in `<main>`, a keystroke in a field drops focus after the first character and a press loses its click when its focus change re-renders the page. The console keeps its landmarks, lists and headings and works around nothing; with `tagName` lower-cased in the browser as a diagnostic, typing, focus and presses behave | in `leptoskit_native`'s web renderer, typed tachys elements (`html::main()`, `html::ul()`, …) for semantic tags, or a case-insensitive tag comparison in tachys; and a browser test that a semantic container keeps its DOM node and focus across a re-render |
| 16 | Text that wraps in a row on the web                                                               | any `Text` beside another child in a `flex-row`: the device page's scopes, the sign-in sheet's list | the layout engine, and so the headless tests and native, measures such a text against the row's remaining width and wraps it; the web renders it with `flex-shrink: 0` at its full width, so a long line runs past the sheet. The console gives those texts `flex-1`                                                                                                                                                                                                                                                                                                                                                                                                                                               | the web renderer giving text the width the layout engine gives it (`min-width: 0` and shrinking text by default, or the measured width), and a browser test comparing a wrapped row against the headless frame                                                |

**Checked, and not gaps**: picking files (`leptoskit_plugin_document_picker`, web and native), clipboard, haptics, QR codes as `Vector` paths, charts as `Vector` paths, connection state and offline queues (`use_live_status()`), virtualised lists (`leptoskit_virtual`), scroll position (`scroll_offset`), transforms and transitions on iOS, macOS, Android and the web, ARIA roles including `Switch`, `RadioGroup`, `SpinButton` and `Status`, real form controls in the generated apps, push notifications, and deep links. One documentation fix falls out of this: the component reference's "Backend availability" still says the generated apps don't draw the form controls.

## 20. What the design needs from the API

The contract's operations (§6) cover most screens. These additions are what the design assumes; each names the build PR that needs it.

| #  | Addition                                                                                                                                 | Why                                                               | PR    |
| -- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | ----- |
| 1  | Reserved slugs: the console's route words (§3)                                                                                           | `/acme/settings` must not be a project                            | UI 1  |
| 2  | The signer on each deployment: person or token, and via (web, CLI with host name, token's client)                                        | the signature (§9.2)                                              | UI 2  |
| 3  | `deployments::in_flight` for an organisation, live                                                                                       | the shipping chip, the Releases tab, projects' states             | UI 1  |
| 4  | `projects::environment_status`, live: per process ready/desired, asleep, waking, restarts, wake times, out-of-memory kills, last heard   | topology, health, sleeping and unknown states                     | UI 2  |
| 5  | `deployments::preview_release`: a release's lines, costs and fine print without shipping                                                 | draft sheets for rollbacks, ship again, store releases and GitHub | UI 2  |
| 6  | Per-line progress and stamps in `deployment_events`; build steps with cache hits; the native fingerprint decision; a `reconciling` event | the sheet's lines, the Build station, the `UNKNOWN` state         | UI 2  |
| 7  | `build_logs` as a live query                                                                                                             | the build log streaming in (§8.2)                                 | UI 2  |
| 8  | `create_project` returning what `launch` found (targets, `.env` names, never values); a starter's source for `lk cloud project clone`    | the page coming alive; "Get the code"                             | UI 2  |
| 9  | `billing::plans` with unit prices per size and meter; `usage` with a forecast and `updated_at`                                           | the price as you move (§11.3), the cost chip and meter            | UI 1  |
| 10 | Scale changes as events, and a flag when an environment differs from its last `leptoskit.toml`                                           | "Applied by", "differs from `leptoskit.toml`"                     | UI 3  |
| 11 | Domains: the records required, what DNS answered for each at the last check, and certificate state                                       | the record table (§12.2)                                          | UI 3  |
| 12 | Databases: migrations applied with their release, backup kinds, restore's expected impact, restore to a time                             | §12.3                                                             | UI 3  |
| 13 | Invitations: list pending and revoke                                                                                                     | §14.2                                                             | UI 3  |
| 14 | `billing::invoices`, and what a reached cap put to sleep                                                                                 | §14.4, the cap-reached panels                                     | UI 3  |
| 15 | `stores::check_store` (the store client's dry-run checks on a worker)                                                                    | the doctor (§13.2)                                                | UI 3  |
| 16 | A done state per report step, and marking person steps done                                                                              | §13.4                                                             | UI 3  |
| 17 | Signing requests prepared when the person presses Sign, with the publisher's balance, an expiry, and the decoded transactions            | blockhashes expire in about a minute (§13.6)                      | UI 3  |
| 18 | A `todo` query across organisations, and a push topic per person                                                                         | To do and its notifications (§13.5)                               | UI 3  |
| 19 | GitHub: app installation, repositories, `leptoskit.toml` at a ref, and `git` sources                                                     | "From GitHub" (§8.3); after UI 3                                  | later |
| 20 | Request metrics (rate, latency, errors)                                                                                                  | not designed; the health card says where they live today          | later |

## 21. Build order

Three pull requests, matching the contract's UI slices. Each ships screens that work end to end against the fake control plane, in light and dark, on the web and in the iOS and Android apps, with insta snapshots of the trees and screenshots of each screen at 400px and 1280px attached to the pull request.

**UI 1: the shell and signing in.**

- `leptoskit_design` with the admin's look-only builders moved out of `leptoskit_admin` (§5), then `leptoskit_cloud_app::style` with the `--cloud-*` tokens.
- The shell: top bar, rail, tab bar, the layout grid, `OfflineChip`, keys.
- `/sign-in`, `/sign-in/email`, `/welcome`, `/device`, `/organizations`, `/account`, `/<org>` (projects, with its empty state).
- Components: the sheet primitives, `Stamp`, `TurnCard` with the mint tab, `CommandBlock`, `QrCode`, `EnvRow`, `CostMeter` (the chip), `ShippingChip`.
- Operations: `leptoskit_auth` client functions, `leptoskit_idp` device flow, `account::whoami`, `organizations`, `create_organization`, `projects::projects`, `billing::subscription`, `usage`. Needs §20 #1, #3, #9.

UI 1 as built, and where it differs from the plan above:

- **Blocked in the browser by gap 15.** The pages render on the server and hydrate in Chrome, and their handlers run. Typing and pointer presses fail until `leptoskit_native` keeps semantic elements across renders, because each render replaces the page's `<main>`. The fix is in the renderer, not the console, so UI 1 ships without a workaround.
- **No `leptoskit_design` yet.** The tokens are `leptoskit_cloud_app::style` and the components `leptoskit_cloud_app::components`. Pieces the admin also has keep the admin's names and behaviour (`Tone`, `status_word`, `Kind`, `link_button`, `link`, `raised`, `margin_note`, `mint_tab`, `mark`, `icon`, `env_pill`), so moving them out later is mechanical.
- **Operations that don't exist yet sit behind a boundary.** `data::Deployments` (§20 #3) and `data::Billing` (§20 #9) have one implementation, `NotAvailableYet`. The shipping chip stays hidden, and the cost chip names the plan without a spend. The welcome sheet states plan terms only when billing gives them. `billing::subscription` isn't called: the plan's slug comes from `whoami`.
- **Redirects.** The server answers a signed-out visit to a console page with a redirect to `/sign-in?next=…`. In the browser, a session that ends in place keeps the page and offers to sign in and come back: nothing moves on by itself. Signing out reloads the page at `/sign-in`, so nothing one account loaded stays in memory for the next.
- **Left out, with the reason.**
  - The rail's ORGANISATION pages, and the Releases and To do tabs, wait for their screens (UI 2 and UI 3).
  - `EnvRow` is built but not placed. A project card's environments, production URL and month's cost need operations that don't exist yet.
  - Links to `/<org>/new` and `/<org>/<project>` reach the not-found page until UI 2.
  - On `/device`:
    - "Started 40 seconds ago, near Berlin" needs gap 7.
    - The APPROVED stamp has no time, because the decision doesn't return one.
    - An expired code and a used code read the same, because the server doesn't tell them apart.
  - "Signed in on" lists browser sessions only; terminals signed in with the device flow aren't listed.
  - `/` opens the personal organisation, or the first one, rather than the one opened last.
- **The first-run pages learn about projects in the browser.** Away from an organisation's own pages, its projects (for the rail, `/welcome` and `/device`'s "Your account is ready") wait on `whoami`, and the server can finish a page before such a query answers, which made hydration fail now and then. So only the live page asks: `/welcome` shows its sheet and moves on to the organisation once the projects say the account isn't new, and the rail's project list and the device page's note appear a moment after the page loads.

**UI 2: a new project, the release sheet, and logs.**

- `/<org>/new` (terminal and starter), the release sheet and its stations, drafts (`/ship`), release history, logs, the overview.
- Components: `ReleaseSheet`, `SheetLine`, `StageRail`, `Signature`, `BlockerPanel`, `ListeningChecklist`, `LogView`, `Topology`, `ConfirmPanel`, `StepList` (What to do next).
- Operations: `projects::create_project`, `project`, `environments`, `create_environment`; `deployments::deploy`, `deployment`, `deployments`, `deployment_events`, `cancel_deployment`, `rollback`, `build_logs`, `runtime_logs`; `billing::estimate`. Needs §20 #2, #4–#8.

**UI 3: settings, domains, billing and stores.**

- Scale and cost, variables and secrets, domains, database, project settings, members and invitations, tokens, billing, stores, publishing, the seal, To do.
- Components: `SizePicker`, `Stepper`, `Toggle`, `ChoiceRow`, `Receipt`, `ChangeSlip`, `DnsRecords`, `OnceValue`, `StoreTile`, `DoctorList`, `Seal`.
- Operations: `projects::update_environment`, `delete_environment`, `delete_project`; `config::*`; `domains::*`; `databases::*`; `account::members`, `invite_member`, `accept_invitation`, `set_member_role`, `remove_member`; `tokens::*`; `billing::*`; `stores::*`. Needs §20 #10–#18, and gap 6 for the seal (until it lands, the seal page explains that signing needs the browser wallet support and says `lk publish` will sign with your own keypair once its dApp Store client lands, contract §10).
- The operator views (§15) are plain tables and can follow as a small fourth pull request if UI 3 runs long.

## 22. Open questions

1. **Should the console change scale at all?** `leptoskit.toml` is the source of truth, so a console change drifts from it. **Recommendation**: allow it, show the drift on the process and on the next sheet (§11.4), and later, with GitHub connected, offer "Open a pull request that changes `leptoskit.toml`" instead of applying directly.
2. **What should Pro's spend cap start at?** **Recommendation**: $20, the usage Pro includes, so moving to Pro can't cost more than the plan until someone with billing rights raises it. It's the honest default, and it makes the cap something people meet on purpose rather than discover on an invoice.
3. **Should a change to variables or secrets make its own release?** Today it applies at the next release, which surprises people. **Recommendation**: yes, as a "config release" that skips building like a rollback does, with its own sheet ("Secrets: SMTP_PASSWORD v3 → v4"). Until the contract has one, the slip offers **Ship v42 again with this** (§12.1).
4. **Ship the native apps before fonts reach native (gap 1)?** Signatures would be in the system face. **Recommendation**: yes. Placement, pen-blue ink and "Signed off by" still read as a signature; the hand face arrives with gap 1, and nothing else in the native apps depends on it.
5. **Publish prices on the landing page before anyone has been billed?** **Recommendation**: yes, labelled "early access" with the prices marked as proposed (§17.1), because the receipt is the product's argument and hiding it would make the cloud look like every other "contact us" beta.
