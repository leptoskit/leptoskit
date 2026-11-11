# Exact examples

The example apps from [Exact](https://github.com/expo/exact), rebuilt as leptoskit screens. The game engine is not here. Fixture apps Exact does not list as examples are not here either.

Each screen is one `view!` tree. State that should survive a rebuild uses `remember`.

```bash
cargo run -p exact_examples                 # the names
cargo run -p exact_examples -- caltrain     # that screen, as a screen reader would read it
cd examples/exact && leptoskit dev          # every screen in the browser
```

`leptoskit dev` is the same kind of loop as Exact's `bun host/web/dev.mjs --app <name>`. Open one screen with `?screen=caltrain`. The headless command runs on any platform and does not need a display.

The benchmarks that match the workloads Exact publishes live in `benchmarks/exact`:

```bash
cargo run --release -p exact_bench
```

Exact's published numbers are presented frames on an iPhone, an iPad, and Chrome. This command times how long leptoskit takes to build and commit the same window on the headless renderer.
