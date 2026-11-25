---
"leptoskit_3d": minor
---

# A hand-drawn style: banded light and ink lines

`Scene::set_style` (`Op::SetStyle`) draws a scene like an inked drawing. `Shading::Toon` lights surfaces in 2 to 4 flat bands with antialiased edges, a coloured shade for the darkest band, flat ambient light, a hard highlight and a hard rim, and optional hatching fixed to the surface. `Ink` draws a line around every model as an inverted hull in the same pass: its width is thousandths of the view's height, so it holds on every screen and in each eye, and it can thicken on the shadow side, wobble like a pen, and boil in steps on the scene's clock without asking for frames. Outline normals are welded at load, so hard edges stay closed, and the line follows skins, morph targets, and animation. `Scene::set_ink` (`Op::SetInk`) weighs, colours, or drops a part's line, and a model can weigh its own per vertex in `COLOR_1`'s red channel. `Style::default()` is the physically based look, and scenes draw byte for byte as before without a style. The ops are new, so batches without them encode exactly as before.
