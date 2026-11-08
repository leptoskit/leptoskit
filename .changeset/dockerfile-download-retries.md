---
"leptoskit_cli": patch
---

# Image builds retry their tool download

The Dockerfile that `leptoskit deploy` generates for web apps downloads binaryen's `wasm-opt` from GitHub. It now retries that download up to five times and unpacks it only once it is complete, so a brief GitHub outage no longer fails the image build.
