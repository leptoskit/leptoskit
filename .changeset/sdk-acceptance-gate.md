---
"release": patch
---

# Require generated client acceptance before publication

Every supported SDK now runs its live-server journey on pull requests and before publication. Explicitly selected SDK tests fail when their language toolchain is missing, rather than reporting success without executing. Run the same serial acceptance suite locally with `devenv shell test:sdk`; the development environment includes its toolchains.
