---
"leptoskit_cli": patch
---

# New apps write builder lint to match the starter screens

`leptoskit new` now sets `[lint] markup = "builder"` with `markup_style` and `component_attribute` denied, so a fresh scaffold's builder home screen is clean under `leptoskit lint --deny-warnings`. The skill, creating guide, linting guide, and small-bundles guide match: omitted `render` still defaults to csr in the schema, but new projects keep the explicit ssr they already write.
