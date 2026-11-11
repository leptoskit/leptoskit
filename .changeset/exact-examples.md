---
"release": minor
---

# Exact's examples on leptoskit

The example apps Exact publishes — Caltrain, Shelf, the calendar, the galleries, and the rest of that list, apart from the game engine — now run as leptoskit screens. `cargo run -p exact_examples -- <name>` prints a screen, and `leptoskit dev` in `examples/exact` serves them in the browser.

`cargo run --release -p exact_bench` times the same list workloads Exact publishes: the 10,000-row lists, the coin list, the extra-heavy feed, a Markdown document, and the feed from the Dioxus and web comparisons. The timings are headless build and commit costs on the machine that runs them.
