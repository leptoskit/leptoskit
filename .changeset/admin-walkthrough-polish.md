---
"leptoskit_admin": minor
---

# Honest simulations and Workings you can follow

`AdminBuilder::simulated_infrastructure(true)` tells the admin that its control service's worker runs against a stand-in cloud. Every screen then shows **Simulated** beside the environment pill, Overview and Changes say that plans and applies contact no provider, the plan and apply confirmations say they act on the stand-in, and Learn says a plan was applied on the simulated cloud. It's off by default, so a real admin keeps its real-infrastructure wording. `admin_demo` turns it on unless it runs with `--executor lk`.

The Workings drawer is easier to follow:

- Changing the builder while a drawer is open scrolls the drawer (and only the drawer) to the first line that changed, marks its station **changed**, and says what changed under the drawer's title. Focus stays where it was, reduced motion jumps instead of scrolling, and a live refresh never moves the drawer.
- An in-place page update keeps the drawer's scroll position, the stations you opened, and your focus.
- **Copy** sits in a row above each code block, labelled with the language, so it never covers code.
- On phones, stations start as their headers and open when pressed.
- The Ask station's Rust, SQL, and HTTP tabs are real tabs: `aria-selected`, one tab stop, and the arrow keys, Home, and End move between them.
- Escape and Close give focus back to the control that opened the drawer, such as a plan row.

On Live, a typed event always shows its `struct` badge, even when it arrives after an untyped one.

Simulation wording also covers deploy, first release, maintenance, rollback, health checks, and backup verification. Confirmations and lesson evidence distinguish sample results from real work, and simulated plans never claim to execute the operator CLI. The browser CI suite runs the five Workings and live-event regression checks.

Releases recorded in the same second keep their journal order, so the current release and rollback choices do not depend on random operation IDs.
