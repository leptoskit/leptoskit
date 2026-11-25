---
"leptoskit_3d": minor
---

# Eased transforms and crossfades, and clips from a phase

`Scene::ease_transform` (`Op::EaseTransform`) moves a node from wherever it is to a transform over a number of seconds along an `Easing` (`Linear`, `Smooth` for smoothstep, `In`, or `Out`), asking for frames until it arrives and landing on the exact transform sent; a `set_transform` or another ease takes over from where it has got to. `Play::fade_easing` eases a crossfade instead of blending at a steady rate, and `Play::phase` starts a clip part of the way through, so instances playing one clip need not move in step. Linear crossfades from the start of a clip stay the default. The ops are new, so batches without them encode exactly as before.
