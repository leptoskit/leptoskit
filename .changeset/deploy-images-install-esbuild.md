---
"release": patch
---

# Generated deploy images install esbuild

The Dockerfiles `leptoskit deploy` and `leptoskit infra` generate now install esbuild next to binaryen, pinned to the same release the development environment provides, so a release web client builds inside the image and minifies its JavaScript instead of stopping at `could not run esbuild`.
