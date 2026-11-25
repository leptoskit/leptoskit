---
"leptoskit_3d": minor
---

# Relief-print shading, stencil paper knockout, and camera-space lights

`Shading::Print(Print)` and `Style::print()` add a three-band un-tonemapped relief-print shading mode to `Op::SetStyle`, paired with per-target `Op::SetPrint` (`Scene::set_print`) overrides (`Part` > `Material` base name > `All`). Relief-print shading skips PBR textures (preserving `AlphaMode::Mask` cutout alpha) and tonemapping, draws symmetric `fwidth`-antialiased object-space hatching and cross-hatching that fade to their exact mean tone as pitch shrinks, adds turning-edge paper specular and backlit spot rim highlights, modulates key ink width with a smoothstep light-side curve, gives each part a deterministic IEEE CRC-32 wobble seed with per-figure 3-slot boil, and renders a single-pass stencil paper knockout (`Depth24PlusStencil8`) with per-figure stencil references so overlapping figures separate while attached parts have no interior seam and open single-sided sheets frame cleanly. Scenes with no active knockout stay on `Depth32Float` and draw byte-for-byte unchanged. `Light::camera_space` rotates the directional light per view, and `Asset::from_slice_with_options` (`LoadOptions::outline_smooth_rings` / glTF `extras.outlineSmoothRings`) optionally smooths welded outline normals across up to 8 vertex rings.
