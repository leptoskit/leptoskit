---
"leptoskit_3d": minor
---

# Textured models packed by gltfpack

Scenes read `KHR_texture_transform`, so a textured model compressed by `gltfpack` draws its textures where they belong. gltfpack quantizes texture coordinates and records the scale and offset that undo it as a texture transform; scenes used to ignore it, and every texture was sampled from its corner. The transform is applied to a primitive's texture coordinates as the model loads. A material's textures share one transform, the first found starting from the base color texture, which is what gltfpack writes.
