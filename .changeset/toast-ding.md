---
"leptoskit_native": minor
"leptoskit": minor
---

# Ding-inspired toast pills

Status toasts arrive as a 44px shape, spring open into a pill, morph in place when a newer message arrives, and shrink before they leave — the same phase model and spring tunings as [Ding](https://github.com/YusufCeng1z/ding). Hold a `Toaster`, call `success` / `error` / `warning` / `info` / `loading`, rebuild from `toaster.state()`, and advance phases with `poll`. `ToastViewport` draws the stack from shared builders, so every backend that already draws `View`, `Text`, `Button`, and `Vector` can show a toast.
