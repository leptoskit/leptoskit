---
"release": minor
---

# Read every sample as markup or builder calls

Code samples on leptoskit.dev that build a component tree now have a Markup and Builder switch. Markup shows the tree as `view!` markup, as before; Builder shows the same tree as builder calls, like `View().class("p-4").child(Text("Hello"))`. One switch changes every sample on the page, and the choice is remembered on the landing page and throughout the docs. It applies before the page first draws, so a sample never flickers from one style to the other, and pages read as markup without JavaScript. Copy copies the style that is showing.

`leptoskit_docs` derives the builder form when it builds a book: each Rust sample with leptoskit `view!` markup gets both forms, laid out the way rustfmt lays out code, and leptos's own `view!` is left alone. Markup that has no builder form fails the build at its line and says why; write `markup-only` after the language, as in `` ```rust markup-only ``, to show a sample as written.
