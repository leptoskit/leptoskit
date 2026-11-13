---
"leptoskit_cli": patch
---

Release web builds minify the JavaScript glue, its snippets, and the chunk loader with esbuild, next to `wasm-opt`. Export names and `import.meta` stay, so the page's imports keep working. `esbuild` comes with the development environment; a machine without it can set `bundle.wasm_opt = false` to skip both.
