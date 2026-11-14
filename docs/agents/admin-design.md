# Admin design direction

Design direction for `leptoskit_admin`, the one admin web app that holds **Analytics**, **Infrastructure**, and **Learn**. It replaces the look of the deployment console (`crates/leptoskit_deploy_console/src/control/dashboard.rs`) and the analytics example page. Domain detail is in [analytics-service.md](analytics-service.md) (§1, §5, §8, §10, §14, §18) and [deployment-control-plane.md](deployment-control-plane.md). The brand is in [`docs/src/brand.md`](../src/brand.md).

A clickable prototype of this direction was built outside the repository (images and HTML aren't committed). The owner has a copy; ask for it rather than re-deriving the screens from prose.

## 1. Concept: show your workings

Every admin has charts and buttons. Ours **shows its workings**, the way a maths teacher asks you to: every number on screen can be walked back to the code that made it, and every infrastructure button can be walked forward to the change it will make. You learn leptoskit by using the admin, not by leaving it to read docs.

One rule carries the whole idea, and it comes straight from the logo:

- **Ember is your app.** Ember is the data your app produced and the actions you take on it. The first chart series is ember, primary actions are ink with an ember accent.
- **Mint is the machinery.** Mint means "this opens how it works". The mint piece in the logo is the part that arrives over the air; in the admin, mint is the part that arrives from underneath. Mint is never used for data and never for status.

If something is mint, you can press it and learn something. That's the whole visual contract.

### The signature interaction: Workings

Every chart card, every stat, every infrastructure action carries a small **mint tab** in its top-right corner: the logo's mint piece, tilted −12°, with the word `Workings`. Press it (or focus the card and press `W`) and the **Workings drawer** opens beside the card.

The drawer is always **ink**, in both themes: you're looking at the underside of the page. It shows the card's whole line as five **stations**, top to bottom, joined by a mint rail with a dot at each station:

| Analytics card                                                                                                                                               | Infrastructure action                                                                                  |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------ |
| **Emit**: the Rust type that produces the event (`#[derive(analytics::Event)] struct CheckoutStarted { … }`) and the call site (`src/routes/checkout.rs:42`) | **Declare**: the `leptoskit.toml` change, as a diff (`instances = 1` → `2`)                            |
| **Send**: what crosses the wire for one event, with bytes (typed: 19 bytes; the same event as JSON: 214 bytes)                                               | **Generate**: the fragment of `infra/<name>/main.tf.json` that `lk infra generate` writes for it       |
| **Store**: the table and columns it lands in (`events`, `sessions`, the context row)                                                                         | **Plan**: the OpenTofu plan as a resource diff: `address`, `actions` (create, update, delete, replace) |
| **Ask**: the query, as the typed `Query::Trends(TrendsQuery { … })` value, the SQL it renders to, or the HTTP request; three tabs                            | **Apply**: the command the control plane runs (`lk infra apply`) and the operation events it streams   |
| **Draw**: the rows the query returned and how they became this chart                                                                                         | **Check**: what "done" means: the health checks, smoke test, and backup verification that gate it      |

Analytics and infrastructure share the five-station shape on purpose: once you've opened one drawer, every other drawer reads the same way.

Three things make this more than a "view SQL" button:

1. **The workings are live and two-way.** Change the breakdown in the trends builder and the `breakdown: Some(Breakdown::property("$browser"))` line in the Ask station changes, with the changed lines washed mint for 1.2 s. Change `instances` in the infrastructure form and the Declare diff and the Plan update. You learn the API by clicking the UI. Every station has **Copy** (as Rust, SQL, TOML, or `curl`), so the drawer is also how you take a query into your own code.
2. **Linked highlighting.** Pressing a series in the chart highlights the struct field and the SQL column that produced it; pressing a resource in the Plan highlights the TOML key that caused it. The rail draws a mint thread between them.
3. **The Send station sells the design.** Showing the bytes on the wire for a typed event next to the JSON equivalent teaches why typed events exist (§5 of the analytics design) without a paragraph of prose.

The drawer is honest about what it doesn't know. An untyped `capture("name", props)` event shows "This event has no Rust type yet" in Emit, with the derive you could write, generated from the properties seen so far. An action the console can't explain (an imported resource) says so.

**Workings mode.** A switch in the top bar, **Show workings** (`Shift+W`), labels every card's mint tab with its source (`CheckoutStarted → Trends`, `infra.hosts.web.server_type`). It's on by default for an admin's first week and remembered per person. Off, the tabs collapse to the bare mint piece. This is the X-ray view: turn it on and the whole page tells you where it comes from.

### Why not a PostHog look

PostHog's personality is a cartoon hedgehog and dense grey chrome. Grafana and Vercel are black glass. We're warm **paper and ink** from the leptoskit brand: an off-white page, near-black ink, hairline rules, one ember accent, and mint only for the machinery. The density is calm: fewer, larger cards with real numbers, not twenty sparkline tiles. The fun is in the details (the mint piece, the handwritten margin notes, the mark that assembles as you learn), not in decoration.

## 2. Pushback on the brief

- **"A Learn hub" as a third area is the generic part.** A docs mirror inside the admin will be ignored. Learning happens in place (Workings, margin notes, empty states that listen, short tours on your own data). The Learn hub is a progress map that links into those, plus short lessons that are verified against your real project. It doesn't duplicate the public book; it links to it for depth.
- **"Saved insights" and "Dashboards" should be one library.** An insight is saved once; a dashboard is a named arrangement of saved insights. The library has two tabs, not two areas.
- **The web analytics overview is the default dashboard**, not a separate concept: same cards, same Workings, just one that every project gets without setup (§14).
- **Live events shouldn't be buried.** A live pulse (events per minute, a 60-second sparkline) sits in the top bar on every screen. Pressing it opens Live. It also tells you at a glance that ingestion is working.
- **Fleet APM belongs to a project, not a separate section.** Request rate, p95, and error rate per route are a project's **Health** tab, next to the release that caused any change.
- **Don't build per-step approval for routine infra.** Following the control plane design, ordinary deploys inside policy just run. The plan review screen is for plans that touch persistent resources, exceed budget, or the admin asked to review.

## 3. Information architecture

```text
Top bar   mark · App name · env pill · project switcher · ⌘K · live pulse · Show workings · theme · avatar
Rail      ANALYTICS                    INFRASTRUCTURE              LEARN
          Overview                     Projects                    Your progress
          Live                         └ <project>                 Lessons
          Insights  (Trends,             Overview                  Glossary
                     Funnels,            Changes (plan → apply)
                     Retention)          Releases
          Library   (Saved, Dashboards)  Operations
          People                         Health
                                         Backups
                                         Config
```

- **Routes**: `/_leptoskit/admin/analytics/overview`, `…/analytics/live`, `…/analytics/insights/new?kind=trends`, `…/analytics/insights/<id>`, `…/analytics/library?tab=dashboards`, `…/analytics/dashboards/<id>`, `…/analytics/people`, `…/analytics/people/<person_id>`, `…/infra/projects`, `…/infra/projects/<id>/<tab>`, `…/infra/operations/<op_id>`, `…/learn`, `…/learn/<track>/<lesson>`. Every screen is a deep link; the Workings drawer adds `#workings=<card_id>` so a link can open a card with its workings showing (for teaching: "look at this").
- **⌘K** finds screens, saved insights, people (by distinct id or email property), events in the catalogue, projects, operations, and lessons. Typing an event name offers "Trend of <event>".
- **Environment pill**: `production` is ember-filled, `staging` outlined, `development` dashed ("development admin, loopback only" when `lk dev` signs in without auth). The env pill is the one place ember is used as a fill outside data: it's a reminder that actions are real.

### Layout grid

| Width                   | Rail                                               | Content                                   | Workings drawer                                                                                                                                     |
| ----------------------- | -------------------------------------------------- | ----------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| ≥ 1280px (`xl`)         | `w-60` full rail with labels                       | 12-column grid, `gap-4`, max width 1200px | splits the screen, drawer `w-[440px]` on the right; while it's open the rail collapses to icons and the page reflows to its 6-column layout (below) |
| 1024–1279px (`lg`)      | `w-60`                                             | 12 columns                                | overlays the content from the right, `w-[440px]`                                                                                                    |
| 640–1023px (`sm`, `md`) | `w-16` icon rail, labels in a tooltip              | 6 columns                                 | overlays, full height, `w-full max-w-[440px]`                                                                                                       |
| < 640px                 | none; a bottom tab bar with the three areas and ⌘K | 1 column, `px-4`                          | a bottom sheet at 85% height, stations collapsed to headers                                                                                         |

The compiler rejects `fixed` and `sticky` positioning, so the shell is a full-height flex column (`h-screen flex flex-col`): top bar, then a row of rail and an `overflow-scroll` content view, then (on phones) the tab bar. Pinned chrome comes from the layout, not from positioning.

Card spans at 12 columns: stat tiles `col-span-3`, standard chart `col-span-6`, wide chart `col-span-12`, top-N lists `col-span-4`. At 6 columns, stat tiles take half a row (`col-span-3`) and every other card takes the full row; at 1 column everything is full width.

**Workings open at 1280px.** 1280 minus a 240px rail and a 440px drawer leaves 600px, which is too narrow for side-by-side cards. So opening Workings collapses the rail to `w-16` icons and switches the page to the 6-column layout; the trends builder becomes a wrapped row of fields above the chart. The prototype confirmed this: without it the chart and builder were cramped and card headers overflowed. Closing the drawer restores both.

**Card headers wrap.** A card header is `flex flex-row flex-wrap items-center gap-2`. In Workings mode the mint tab's source label can be long (`CheckoutStarted → Trends`), so the tab may drop to its own line and its label truncates; the title never truncates.

**Chart text doesn't scale with the plot.** Labels are `Text` views outside the `Vector`, so they stay `text-xs` at any width. (The prototype draws text inside the SVG and had to size each chart's `viewBox` to its card to stop labels growing on wide cards. Don't copy that.)

## 4. Tokens

### What the class compiler allows

Read this before designing interactions. leptoskit's Tailwind compiler is a strict subset (`crates/leptoskit_native/src/tailwind/mod.rs`):

- Variants are only `dark:`, `active:`, `starting:`, and breakpoints. **No `hover:` or `focus:`.** Hover and focus styles are state the component holds and turns into classes (a `focused` signal, a `selected` signal). Pressed feedback uses `active:`.
- **No shadows, filters, `fixed`, `sticky`, or `calc`.** Elevation comes from tone and borders (below).
- **No font-family utility**, and no `tabular-nums`, `tracking-*`, `uppercase`, or `truncate`. These are gaps, listed in §13, that the admin needs.
- Custom colours come from `[theme]` tokens as `bg-[var(--name)]`. `leptoskit_admin` installs its own tokens with `Theme::set_color` at mount, so apps don't have to.

### Colour

Each token has a light value and a `-dark` value. Use them as pairs, for example `bg-[var(--admin-surface)] dark:bg-[var(--admin-surface-dark)]`. Put the pairs in Rust constants in `leptoskit_admin::style` so screens never spell them out.

| Token                    | Light     | Dark      | Use                                                     | Contrast        |
| ------------------------ | --------- | --------- | ------------------------------------------------------- | --------------- |
| `--admin-paper`          | `#faf6f0` | `#16111f` | page background (the brand's paper and ink)             |                 |
| `--admin-surface`        | `#fffdf8` | `#1d1726` | cards, the rail, the top bar                            |                 |
| `--admin-sunken`         | `#f3ede4` | `#251e30` | inputs, table headers, wells, chart plot area in tables |                 |
| `--admin-line`           | `#e8e0d4` | `#30283c` | hairlines, card borders, gridlines                      |                 |
| `--admin-line-strong`    | `#cdbfad` | `#4a3f58` | input borders, focused card, dividers you need to see   |                 |
| `--admin-ink`            | `#16111f` | `#f4efe8` | body text, headings, primary button fill (light)        | 17.2:1 / 16.2:1 |
| `--admin-ink-2`          | `#4d4558` | `#c9c0d0` | secondary text, axis labels                             | 8.5:1 / 9.9:1   |
| `--admin-ink-3`          | `#6e6578` | `#9d93a8` | muted text, timestamps, placeholders                    | 5.1:1 / 6.0:1   |
| `--admin-ember`          | `#e5304f` | `#ff5a73` | env pill, the logo's ember, focus ring                  |                 |
| `--admin-ember-warm`     | `#ff7a45` | `#ff8a5c` | ember gradient top, primary button's offset block       |                 |
| `--admin-mint`           | `#14bf9b` | `#5ef2c2` | the mint tab, the Workings rail, glossary underline     |                 |
| `--admin-mint-text`      | `#0a6e59` | `#5ef2c2` | mint text on paper or wash                              | 5.5:1 / 13.2:1  |
| `--admin-mint-wash`      | `#e3f6ee` | `#11302a` | changed-line highlight, margin note background          |                 |
| `--admin-underside`      | `#16111f` | `#0f0b16` | the Workings drawer (always ink)                        |                 |
| `--admin-underside-line` | `#30283c` | `#2a2236` | drawer rules                                            |                 |

Status colours are reserved for state, always shown with an icon and a word, never colour alone:

| Token          | Light     | Dark      | Icon and word                      |
| -------------- | --------- | --------- | ---------------------------------- |
| `--admin-good` | `#1f7a45` | `#5fd08a` | ✓ Healthy, Succeeded, Create       |
| `--admin-warn` | `#9a5b00` | `#e3b341` | ! Degraded, Waiting, Update        |
| `--admin-bad`  | `#b8233f` | `#ff8a9e` | ✕ Failed, Blocked, Delete, Replace |
| `--admin-info` | `#4d4558` | `#c9c0d0` | • Running, No change               |

All status text clears 4.8:1 on paper and 7.8:1 on the dark surface. Each has a `-wash` at 10% alpha for row backgrounds (`bg-[var(--admin-bad)]/10`).

Plan diffs use status colours plus a symbol and a word: `+ create`, `~ update`, `− delete`, `± replace`, never colour alone.

### Chart palette

Categorical, in this fixed order. Colour follows the entity, never its rank: a filter that removes series 2 doesn't repaint series 3.

| Slot | Hue    | Light (on `#fffdf8`) | Dark (on `#1d1726`) |
| ---- | ------ | -------------------- | ------------------- |
| 1    | ember  | `#e0492b`            | `#e8512f`           |
| 2    | blue   | `#2f6fd6`            | `#3f7fe6`           |
| 3    | green  | `#2a8f3c`            | `#2f9a4a`           |
| 4    | violet | `#8a5cd6`            | `#8f6ee6`           |
| 5    | gold   | `#b57c00`            | `#c08a12`           |
| 6    | pink   | `#d6528f`            | `#d65690`           |

Validated with the dataviz palette checker (OKLab ΔE×100): every slot is at least 3:1 against its surface in both modes; worst adjacent colour-blind separation is 13.2 light and 14.1 dark (target ≥ 8); worst adjacent normal-vision separation is 20.5 light and 21.1 dark (floor 15). The first three also pass the all-pairs normal-vision floor (27.1 / 26.6), but ember and green fall to 6.9 / 6.6 for deuteranopes when every pair is compared, so line charts always carry direct end labels, and a compare period is the same hue dashed, not a new colour. A seventh series folds into **Other** (`--admin-ink-3`); the breakdown control says "Top 6 + Other" and offers small multiples.

Sequential (retention grid, heat tables), one hue, continuous:

| Step      | Light     | Dark      |
| --------- | --------- | --------- |
| 0 (empty) | `#f3ede4` | `#251e30` |
| 1         | `#dde8fa` | `#1f2c48` |
| 2         | `#b9cff3` | `#22396a` |
| 3         | `#8db1ea` | `#2a4d8f` |
| 4         | `#5b8fe0` | `#3a66b8` |
| 5         | `#2f6fd6` | `#5b8ae6` |
| 6         | `#1f53a8` | `#8fb2f2` |

Cell text switches from `--admin-ink` to `#ffffff` (light) at step 4, and from `--admin-ink` (dark) to `#0f0b16` at step 5 in dark mode.

Mint never appears in a chart, and neither do status colours, except the APM error-rate line, which uses `--admin-bad` because it _is_ a status.

### Type

The brand's Skribble Noto Sketch is lovely for the docs but tiring in a dense admin. So:

| Role                           | Face                                           | Size and weight | Classes                                           |
| ------------------------------ | ---------------------------------------------- | --------------- | ------------------------------------------------- |
| Page title                     | system sans                                    | 24px semibold   | `text-2xl font-semibold leading-tight`            |
| Section and card title         | system sans                                    | 16px semibold   | `text-base font-semibold`                         |
| Body, table cells              | system sans                                    | 14px            | `text-sm leading-normal`                          |
| Labels, axis ticks, timestamps | system sans                                    | 12px medium     | `text-xs font-medium`                             |
| Hero number (stat tiles)       | system sans, tabular figures                   | 32px semibold   | `text-3xl font-semibold leading-none`             |
| Code (Workings, TOML, SQL)     | Skribble Mono Playful, fallback `ui-monospace` | 13px            | `text-[13px] leading-relaxed` + mono family (gap) |
| **Margin notes** (Learn)       | **Skribble Noto Sketch**                       | 15px            | `text-[15px] leading-snug` + note family (gap)    |

The handwriting is reserved for teaching. Anything written in the sketch face is the admin talking to you, like pencil notes in the margin of a textbook. Numbers are never handwritten.

### Space, radius, elevation

- Spacing is Tailwind's 4px scale. Card padding `p-4` (`md:p-5`); gap between cards `gap-4`; between page sections `gap-8`; inside a card between title and body `gap-3`.
- Radius: cards `rounded-xl`, controls and inputs `rounded-lg`, chips and pills `rounded-full`, chart bars `rounded-[3px]` on the data end.
- Elevation has three steps, all without shadows:
  1. **Page**: `--admin-paper`.
  2. **Card**: `--admin-surface` with a `border border-[var(--admin-line)]`.
  3. **Raised** (drawer, popovers, menus, the primary button): the brand's **hard offset block**. A sibling `View` positioned `absolute` behind the element at `translate-x-1 translate-y-1`, filled `--admin-ink` (light) or `--admin-mint` for Workings-related raised things. It's the docs' button hover effect, made without `box-shadow`. Pressing a raised button moves it onto its block (`active:translate-x-1 active:translate-y-1`), so it feels physically pressed.
- Focus: a 2px `--admin-ember` ring drawn as `border-2` on a wrapper the focused signal turns on, never removed.

### Motion

Short and purposeful: the drawer slides in from the right over 200ms `ease-out`; changed lines in Workings fade from mint wash over 1.2s; the mint tab tilts from −12° to 0° when pressed ("it lands"); live rows enter with `starting:opacity-0 starting:translate-y-1`. Honour reduced motion: everything becomes an instant change.

## 5. Components

Every component lists its states. "Selected" and "focused" are component state turned into classes, as §4 explains.

| Component                                         | Anatomy                                                                                                                                                                          | States                                                                                                                                                      |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Top bar**                                       | mark, app name, env pill, project switcher, ⌘K field, live pulse, Show workings switch, theme toggle, avatar menu                                                                | pulse: live / paused / disconnected (grey, "Reconnecting…")                                                                                                 |
| **Rail**                                          | three group labels, items with icon + label + optional count                                                                                                                     | item: default / selected (ink text, `--admin-sunken` fill, 2px ember bar on the left) / focused                                                             |
| **Button**                                        | primary (ink fill, paper text, ember-warm offset block), secondary (surface, line-strong border), quiet (text only), danger (bad text, bad border; filled only inside a confirm) | default / pressed / focused / disabled (ink-3 text, no block) / busy (spinner + verb in -ing: "Applying…")                                                  |
| **Mint tab**                                      | the tilted mint piece + `Workings`, label in Workings mode                                                                                                                       | rest / pressed (tilt to 0°) / open (filled mint, ink text)                                                                                                  |
| **Card**                                          | title row (title, period chip, mint tab, overflow menu), body, footer note                                                                                                       | loading (skeleton in `--admin-sunken`) / empty (teaching empty state) / error (message + retry + Workings stays available) / stale ("Paused: 2 min behind") |
| **Stat tile**                                     | label, hero number, delta chip, sparkline                                                                                                                                        | delta up/down/flat with arrow and text ("▲ 12% vs last week"); delta colour is ink, never green/red, because "up" isn't always good                         |
| **Chart frame**                                   | legend row, plot, x axis, y axis, tooltip                                                                                                                                        | idle / scrubbing (crosshair + tooltip) / pinned (tooltip stays, "Show people" action) / empty / partial (dashed last segment for the incomplete period)     |
| **Bar list** (top pages, referrers, UTM, devices) | label, value, share bar behind the row                                                                                                                                           | row pressed → filters the whole page by that value (chip appears in the filter bar)                                                                         |
| **Filter bar**                                    | date range, compare toggle, property filter chips, "test accounts excluded" chip                                                                                                 | chip default / editing / invalid (bad border + reason)                                                                                                      |
| **Table**                                         | sortable header, rows, pagination                                                                                                                                                | row selected, row pressed (navigates), sort asc/desc, empty                                                                                                 |
| **Diff list** (plan)                              | symbol + action word, `address`, resource type, the changed attributes collapsed                                                                                                 | per action colour; "blocked" rows carry the typed blocker                                                                                                   |
| **Timeline**                                      | vertical rail with step dots, event title, time, duration, progress bar, log excerpt                                                                                             | pending / running (animated dot) / succeeded / failed / skipped / waiting for approval                                                                      |
| **Workings drawer**                               | header (card title, close), station rail, station sections, footer (copy all, open docs)                                                                                         | closed / open / station collapsed / changed lines highlighted / "no type yet"                                                                               |
| **Margin note**                                   | sketch-face text on mint wash, a mint pencil line on the left, "Got it"                                                                                                          | shown / dismissed / retired (after the thing is used 3 times)                                                                                               |
| **Glossary term**                                 | dotted mint underline                                                                                                                                                            | pressed → popover with a one-line definition, an example, and "Show workings"                                                                               |
| **Listening empty state**                         | what's missing, the exact code to add, a live checklist that ticks itself                                                                                                        | waiting (pulsing dot) / received ("First event arrived 2s ago") / timed out (troubleshooting steps)                                                         |
| **Env pill**                                      | env name                                                                                                                                                                         | production / staging / development                                                                                                                          |
| **Confirm sheet**                                 | what will happen in one sentence, the Workings summary, type-to-confirm for destructive actions, second-factor prompt when required                                              | ready / needs second factor / busy                                                                                                                          |

## 6. Charts

All charts are `Vector` + `VectorPath` paths computed in Rust from the query result, with labels as `Text` views laid out around the plot (vectors don't draw text). Strokes use `StrokeScaling` non-scaling so they keep their width when the plot stretches. A plot is a `viewBox` in data units mapped to a fixed aspect (`aspect-[2/1]` standard, `aspect-[4/1]` wide).

Shared anatomy: four horizontal gridlines in `--admin-line`, no vertical gridlines; y labels at the left in `--admin-ink-3`, `text-xs`; x labels only at the start, the middle, and the end of the range; no chart borders. Axes start at zero for bars and areas. Values use SI suffixes (`12.4k`), durations `1m 32s`, percentages one decimal.

**Interaction (scrubbing).** There's no `hover:` and no pointer-move without a press, so charts use a **scrub**: press anywhere on the plot and drag (a `Pan` gesture) to move a crosshair; release to pin the tooltip at that point. On the web, the renderer can also follow the mouse when a hover pointer event is available (gap in §13). A pinned tooltip offers **Show people** (the `Actors` query behind that point, §10) and **Workings**. Hit targets are the full column, not the 2px line.

| Chart                                  | Spec                                                                                                                                                                                                                                                                                                                                                                       |
| -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Line / area** (trends)               | 2px round-capped lines, one per series in palette order; area fill at 12% of the series colour only when there's one series; the current, incomplete period is dashed (`dashes(&[4.0, 4.0])`); compare period is the same hue dashed at 60% opacity; direct label at the right end of each line (name + last value) when there are ≤ 4 series, otherwise the legend only   |
| **Bars** (trends by day, top-N)        | columns with a 2px gap, `rounded-[3px]` on the data end, anchored at zero; stacked only when the total matters, with a 2px surface gap between segments                                                                                                                                                                                                                    |
| **Bar list**                           | horizontal share bars in `--admin-sunken` behind text (`text-sm`), value right-aligned; the bar is the only colour, at series 1 15% tint, so the list stays calm                                                                                                                                                                                                           |
| **Funnel**                             | one column per step, left to right, height = people at that step; above each, the step's count; between columns, the conversion arrow with `↘ 62.4%` and the median time to convert; the lost part of each column is hatched (`--admin-line-strong` 45° lines) so "dropped off" is visible without colour. With a breakdown, steps become grouped columns in palette order |
| **Retention grid**                     | rows are cohorts (start date + cohort size), columns are periods 0…N; cells are `rounded-[3px]` squares in the sequential ramp with the percentage inside; period 0 is always 100% and drawn in `--admin-sunken` so it doesn't dominate; a mean row on top, bold                                                                                                           |
| **Sparkline**                          | 1.5px line, no axes, final point as a 6px dot; used in stat tiles and the live pulse                                                                                                                                                                                                                                                                                       |
| **Latency** (APM)                      | p50 and p95 as lines (slots 2 and 1), the SLO as a dashed `--admin-ink-3` threshold with a text label; error rate is a separate chart below on its own axis (never a dual axis) in `--admin-bad`                                                                                                                                                                           |
| **Meters** (host memory, disk, budget) | a horizontal bar in `--admin-sunken` with the used part in ink, a threshold tick at 80%, used/total as text; over threshold the text gains `!` and turns `--admin-warn`                                                                                                                                                                                                    |

Every chart has a **Table** toggle in its overflow menu that shows the same rows (accessibility and copy-paste), and identity is never colour alone: legends, direct labels, and dash patterns carry it.

## 7. The Learn system

Teaching shows up in six places, from the least to the most intrusive. The rule: **teach in place, on the admin's own data, and stop teaching once they've got it.**

1. **Glossary terms.** Domain words (bounce rate, distinct id, session, p95, plan, drift, replace, first release, maintenance) carry a dotted mint underline the first few times they appear on a screen. Pressing one opens a popover: one-sentence definition, a concrete example from this project ("A bounce is a session with one pageview and under 10 seconds. 41% of yesterday's sessions bounced."), and a link to the station of the Workings that computes it.
2. **Margin notes.** Short notes in the sketch face on a mint wash, beside a new or tricky control: "Breakdowns split each line by a property. Try `$browser`." Each has **Got it**. Notes retire on their own after the person uses the control three times, so they never nag. At most one note per card and three per screen.
3. **Workings.** The signature (§1). Every station starts with one plain sentence of why ("Typed events send no property names, so this event is 19 bytes instead of 214").
4. **Listening empty states.** An empty chart never says "No data". It says what's missing, shows the exact code to add with a Copy button, and **listens**: a live checklist ("Add the analytics plugin ✓, Capture an event … waiting") ticks itself when the first matching event arrives, then the chart draws in. Infrastructure does the same for "no project yet": the `[infra]` table to add, then it detects the TOML and offers **Plan**.
5. **Three-stop tours.** Each area has one tour of exactly three stops, on the person's own data: Analytics ("This is a visitor → this line is a query → open its workings"), Infrastructure ("This is what's serving → this is a change → this is how you undo it"). A stop is a margin note anchored to a real widget plus one thing to do; it advances when they do it. Tours start from an empty state or the Learn hub, and auto-start only once ever, on first sign-in, as a single dismissible offer.
6. **Lessons** in the Learn hub (below), for the bigger ideas.

Blockers teach too. A typed control-plane blocker (`budget_exceeded`, `schema_incompatible`, `runner_unavailable`, `persistent_resource_deletion_blocked`) is shown as: what happened in a sentence, why it's a rule, the fix, and the Workings of the plan that tripped it.

### The Learn hub

Three tracks, each a short line of lessons (3–6 minutes each):

- **Analytics**: Your first typed event · Reading a trend · Building a funnel · Retention, honestly · Who are these people? · SQL when you need it
- **Infrastructure**: What `[infra]` describes · Plan, review, apply · Releases and rollbacks · Backups you've actually restored · Reading health
- **Build with leptoskit**: Builder components · Tailwind in leptoskit · Typed RPC and live queries · Shipping to native with code push

Each lesson is a page with a short explanation, a "do it" step in the real admin (opens the screen with a margin note waiting), and an **outcome check verified from real state**: "You saved a funnel with 3 steps ✓", "Your last backup restored in 41s ✓". A lesson can't be ticked by reading.

Progress is drawn as the **leptoskit mark assembling**: finish the Analytics track and the first ember box appears; Infrastructure, the second; Build, and the mint piece lands at −12°. It's the one celebratory moment in the product, and it's the brand's own story (layout boxes, then the piece arriving over the air).

## 8. Voice

- Plain, second person, present tense. "You have 3 changes to review", not "3 pending modifications detected".
- Lead with the number or the outcome, then the reason. "Checkout conversion fell to 31% (from 38%) after Tuesday's release."
- Name things by what they are in the code: `CheckoutStarted`, `infra.hosts.web`, `lk infra apply`. Code in code formatting, always copyable.
- Buttons are verbs with objects: **Plan changes**, **Apply 4 changes**, **Roll back to v41**, **Verify last backup**. Never "Submit", "OK", "Execute".
- Destructive confirms say exactly what's lost: "This deletes the `notes-backups` bucket and its 92 backups. It can't be undone."
- Errors say what happened, why, and what to do. No apologies, no blame, no exclamation marks. Success is quiet: a check and a past-tense verb ("Applied in 2m 14s").
- Margin notes may be a little warm ("Nice. That's your first funnel."), never cute.

## 9. Analytics screens

All analytics screens share the filter bar (date range, compare, property filters, test accounts excluded) under the page title. A filter set on Overview carries to Insights when you open a card.

### Overview (web analytics)

- **Row 1, stat tiles** (`col-span-3` each, or 2×2 at `md`): Visitors, Pageviews, Sessions, Bounce rate; plus Session duration as the fifth tile on a second row at 1280 only if room, otherwise in the visitors card's footer. Each: hero number, delta vs the previous period, sparkline.
- **Row 2**: Visitors over time (`col-span-8`, line, current vs previous dashed); Channels (`col-span-4`, bar list: Direct, Organic search, Referral, Social, Email, Paid).
- **Row 3**: Top pages (by `$lk_route` template, with a toggle to raw path), Top referrers, UTM (tabs source/medium/campaign) — `col-span-4` each, bar lists.
- **Row 4**: Devices (tabs device type, browser, OS), Countries, Core Web Vitals (LCP, INP, CLS at p75 with good/needs work/poor status words) — `col-span-4` each.
- Pressing any bar-list row filters the whole page. Every card has Workings; the Visitors card's Ask station shows `Query::WebOverview(WebQuery { … })` and the sessions SQL.
- Empty state: the listening state for `$pageview`, with the two lines that turn on the browser module.

### Live

- A full-width stream, newest at the top: time (relative, absolute on press), event name (typed events show a small `struct` badge), person (distinct id or email), route, source icon (web, iOS, Android, server…), and key properties.
- Above it: a filter field using §11's filter syntax with autocomplete, a rate readout ("38 events/s, showing 1 in 4" when sampled to a readable rate), and **Pause** (keeps the place; resume catches up with a "212 new" bar).
- Pressing a row opens the event on the right: all properties grouped (event, context, person), and Workings for that single event (Emit shows its struct; Send shows the actual bytes of this event's batch entry).
- At 400px: rows become two lines (event + time, person + route).

### Trends (insight builder) — with Workings open

- Left (or top on phones), the **builder**: series list (each: event or action, math: total, unique people, per-user average, property sum/avg/p90; filters; a rename), **Add series**, formula (`A / B`), breakdown (property, top 6 + Other), interval (hour, day, week, month), compare to previous period, chart type (line, bar, area, number).
- Right, the chart (`aspect-[2/1]`), the legend, and below it a compact table of the series totals.
- Header: insight name (editable), **Save**, **Add to dashboard**, mint tab.
- With Workings open at 1280, the screen splits: builder collapses to a summary strip above the chart; the drawer takes 440px. Changing any builder control updates the Ask station with the changed lines washed mint.

### Funnels

- Builder: steps (2–20, each an event or action with filters, drag to reorder), conversion window (default 14 days), order (ordered, strict, any), exclusion steps, breakdown, and attribution for the breakdown.
- Chart: the funnel columns (§6) with overall conversion as a hero number above ("31.2% of people who started checkout paid"), and below the chart a step table: step, people, conversion from previous, from first, median time to convert, and **Show people who dropped off** per step (opens the People list filtered by that Actors query).
- Margin note on the first visit: "Strict order means no other events in between. Most funnels want _ordered_."

### Retention

- Builder: start event, return event, first-time or recurring, period (day, week, month), number of periods.
- The retention grid (§6), cohorts newest at the bottom, and above it a line chart of the mean curve with the previous period dashed. Pressing a cell opens the people in it.

### People and person profile

- **People**: a table of persons (avatar initials, email or distinct id, first seen, last seen, sessions, country, top properties chosen by the admin), a search field, and cohort chips. Anonymous-only persons are hidden by default with a toggle ("Include 12,481 anonymous").
- **Person profile**: a header (name or email, distinct ids as chips, first seen, last seen, identified badge), then three tabs: **Timeline** (events grouped by session, each session a collapsible block with duration, entry route, and event count; typed events show their struct name), **Properties** (a table with each property's last set time and which event set it; `$set_once` marked), **Sessions** (list with duration, pageviews, entry and exit route, and a replay link when one exists). Destructive **Delete person** sits in the overflow menu with a type-to-confirm and the second-factor rule.
- Margin note: "Events keep the person they were written with. Merges apply through overrides, so history updates without rewrites." with a glossary link to _merge_.

### Library (saved insights and dashboards)

- Two tabs. **Insights**: a list (name, kind icon, the event it's about, last viewed, owner), search, filter by kind. **Dashboards**: cards showing each dashboard's name, description, number of insights, and a tiny thumbnail of its layout grid (not chart screenshots: just the shaded grid of card spans).
- New dashboard starts with **Web analytics** as a template.

### Dashboard

- A grid of saved insight cards on the 12-column grid, each `col-span-4`, `6`, or `12` (the owner chooses; no free-form drag-resize in phase 1, it's fiddly and generic). Cards are live (§10): a small dot in the card header pulses when it re-ran.
- Header: name, description, date range override for the whole dashboard, **Edit layout** (reorder by drag, set span), **Share** (copies a link to the admin route).
- Workings mode on a dashboard is the best teaching view in the product: every card labels its event and query kind.

## 10. Infrastructure screens

### Projects

- A list of projects as cards: name, environment pills, what's serving (release `v42`, image tag or commit, age), health word with icon, monthly estimate vs budget meter, and the last operation's one-line status. Press → project overview.
- Empty: listening state for `[infra]` in `leptoskit.toml`, with the minimal managed preset table and **Plan** once the file is detected.

### Project overview

- **Now serving** strip: release, image digest (shortened, copyable), commit, deployed by, when; health word; **Deploy…**, **Roll back…**, overflow (maintenance, health check, verify backup).
- **Topology** card: hosts, the tunnel, buckets, the database, drawn as a simple left-to-right diagram from `[infra]` (boxes and lines in `Vector`), each node showing its status word. Pressing a node opens its Workings at the Declare station (its TOML table).
- **Recent operations** timeline (last 5) and **Budget** meter card (fixed charges, estimate, unknowns stated as such: "Excludes tax, traffic, and storage").

### Changes: plan → review → apply

The heart of infrastructure, and where Workings is open by default the first three times.

1. **Plan**: **Plan changes** runs `lk infra plan` as an operation. While it runs, the timeline shows progress events; the page doesn't block.
2. **Review**: a summary sentence ("4 to create, 1 to update, 0 to delete") then the **diff list** grouped by resource type, each row `+ create  hcloud_server.web[1]  cx23 · hel1`. Expand a row for changed attributes as `before → after`. Rows touching persistent resources (primary hosts, volumes, buckets, networks) carry a lock icon; deletion or replacement of them shows the `persistent_resource_deletion_blocked` blocker inline and disables Apply, with the reason and the fix.
3. **Apply**: **Apply 5 changes** (primary). The confirm sheet repeats the summary, shows the fingerprint check ("Plan matches the configuration, provider lock, and state it was made against ✓"), and the budget delta. A failed or interrupted apply says plainly that the plan is used up: "Plan again before retrying."

The Workings drawer here shows Declare (the TOML diff since the last apply), Generate (the `main.tf.json` hunk), Plan (the same diff, as OpenTofu prints it), Apply (the command and its streamed events), and Check.

### Deploy, first release, maintenance, rollback, health check, backup verify

Each is a **sheet** opened from the project, with the same shape: one sentence of what will happen, the inputs, Workings, and a primary verb.

| Action            | Inputs                                                                   | What the sheet explains                                                                                                             |
| ----------------- | ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| **Deploy**        | image tag _or_ git commit (segmented control), with recent builds listed | health gates it'll wait for; that old browser assets are kept                                                                       |
| **First release** | image, migrations to run (listed), admin account to seed                 | that this creates the database and runs every migration; how long it usually takes                                                  |
| **Maintenance**   | on/off, message shown to visitors, expected duration                     | that live connections will be told to reconnect later                                                                               |
| **Roll back**     | target release from the recorded compatible list                         | the three kinds of recovery (app, infra, data); that this only moves traffic to an older image and doesn't reverse database changes |
| **Health check**  | none                                                                     | each probe it runs and what "healthy" means for each                                                                                |
| **Verify backup** | which backup (latest by default)                                         | that it restores into a scratch database and runs checks; the max age rule from `[infra.backups]`                                   |

### Operation timeline

`/infra/operations/<op_id>`: header with the operation kind, who started it, state word, duration, and **Cancel** only when cancelling is safe. Then the vertical timeline: each progress event with time, duration, and an expandable bounded log excerpt; the running step shows a progress bar when the event carries one. The CLI command to follow the same operation (`lk console operations follow <id>`) is shown in Workings, so people learn that closing the tab cancels nothing.

### Health (fleet APM)

- Stat tiles: request rate, p95 latency, error rate, active live connections.
- Latency chart (p50, p95, SLO line) and error-rate chart beneath it, with **release markers**: a thin vertical ink line with the release label at each deploy, so "did the deploy do this?" is answered by looking.
- Routes table: route template, requests/min, p95, error rate, with a sparkline per row; sorted by p95 by default.
- Hosts: per host, memory and disk meters (§6) and CPU sparkline; the Postgres connection budget meter.

### Backups and Config

- **Backups**: schedule from `[infra.backups]`, last success, last verified restore (and how long it took), retention counts as a small table, **Verify backup**.
- **Config**: the project's `[infra]` table rendered read-only with each key explained by a glossary popover, and a "where this lives" note: "Edit `leptoskit.toml` and push, or change it here to open a plan." Changing a value here never writes silently: it opens a plan with the Declare diff shown.

## 11. Sign-in

- A centred card on paper (`max-w-sm`): the mark, "<App name> admin", one primary button **Continue with <App name>** (with the app's icon if configured), and one line: "You'll sign in on <app domain> and come straight back here." No key field.
- After the redirect, if the account lacks an admin role: "You're signed in as ana@example.com, but this account isn't an admin." with **Switch account** and a collapsed "How admins are chosen" note showing the app's `[admin] roles` and the `Roles` hook (that's teaching, too: the person who sees this is often the developer).
- `lk dev` without auth configured: the card says "Development admin, loopback only" and the button says **Enter as development admin**; the env pill is dashed `development`.
- Apps without accounts (the documented `LEPTOSKIT_ADMIN_KEY` fallback) get a single secret field under a "This app has no accounts" heading. It's a fallback, not the design.
- Second factor: when a destructive action needs it, the confirm sheet asks inline; sign-in itself is the main site's job.

## 12. First run

On the first sign-in, the Overview opens with a single offer at the top: "Take the 3-stop tour" or "I know my way around". Workings mode is on. If there's no data yet, the Overview _is_ the listening empty state. The Learn hub's mark starts empty. That's all; no modal carousel.

## 13. Gaps the implementation needs

These don't exist in leptoskit today and the design depends on them. Each is small; build them in `leptoskit_native` (with tests and docs) rather than working around them in the admin.

1. **Font families**: `font-sans`, `font-mono`, and a theme-driven family (`font-[var(--admin-font-note)]`) for code and margin notes; plus `tabular-nums` for numbers in tables and stat tiles.
2. **Text helpers**: `truncate` (single-line ellipsis) for table cells and long event names; `tracking-*` is optional.
3. **Hover pointer events** on the web renderer (pointer move without a press), so charts can follow the mouse; the scrub interaction works without it.
4. **A code block** that the Workings drawer can use: monospaced, line numbers, per-line background (for the changed-line wash), and copy. A small admin widget is fine if it doesn't belong in core.

## 14. Implementation notes

- Build screens from the page-document widgets in analytics-service §18 where they fit (stat, table, chart, form, filter bar, live tail, timeline); Workings is a property of the widget, not of the page: every widget's source (`op`, `args`) already says what to show in Ask, and the event catalogue says what to show in Emit.
- Put class constants for tokens and recurring pieces in `leptoskit_admin::style` (`CARD`, `CARD_TITLE`, `BUTTON_PRIMARY`, `MINT_TAB`, `UNDERSIDE`, …), each a light and dark pair, so no screen spells a colour.
- Keep the 400px layout honest: every screen in §9–§11 must be usable at 400px, with tables becoming two-line rows rather than scrolling sideways, except the retention grid and plan diffs, which scroll horizontally inside their card.
