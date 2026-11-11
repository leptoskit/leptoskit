---
"leptoskit_testing": patch
"leptoskit_cli": patch
---

An Android pinch from `leptoskit device` is two pointers again. Two `input swipe`s each own one pointer, and on API 35 the activity received only one of them while the step still reported success. The pinch now registers one touchscreen with `uinput` and moves both fingers in the same report.
