---
"leptoskit_3d": minor
---

# Eased transforms and crossfades, and clips from a phase

`Scene::ease_transform` (`Op::EaseTransform`) moves a node from wherever it is to a transform over a number of seconds along an `Easing` (`Linear`, `Smooth` for smoothstep, `In`, or `Out`), asking for frames until it arrives and landing on the exact transform sent; a `set_transform` or another ease takes over from where it has got to. `Play::fade_easing` eases a crossfade instead of blending at a steady rate, and `Play::phase` starts a clip part of the way through, so instances playing one clip need not move in step. Linear crossfades from the start of a clip stay the default. The ops (`EaseTransform` tag 25, eased/phased `Play` tag 26) are additive `VERSION = 1` ops: batches without them encode identically and stay compatible with older v1 hosts, while batches containing them require a matching host/renderer version (older v1 hosts reject the batch as an unknown op).
