# Exact workloads

Headless timings for the lists Exact publishes under `bench/`: the 10,000-message list, the rich feed, the 5,000-coin list, the 3,000-row extra-heavy feed, a Markdown document, the feed Exact compared with Dioxus, and the same feed the web shootout scrolls.

```bash
cargo run --release -p exact_bench
```

The table is a host prepare cost. It is not a presented frame rate.
