---
"leptoskit_3d": minor
---

# Additive light for glows, flashes, and sparks

`Scene::set_blend(node, part, Blend::Additive)` (`Op::SetBlend`) makes a part of a model add its colour to what is drawn behind it, scaled by its opacity, instead of covering it. A muzzle flash, a spark, or a soft halo then reads as light over a camera picture, and black adds nothing, so a round glow needs no cut-out. `Blend::Normal` puts it back. Additive parts draw with the transparent parts. The op is new, so batches without it encode exactly as before.
