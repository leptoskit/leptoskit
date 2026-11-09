---
"leptoskit_xr": minor
---

# Distinguish measured and estimated depth

`Frame::depth` now carries `Option<DepthSource>`: `None`, `Measured`, or `Estimated`. The host tags depth from the camera model separately from platform depth, including on phones with AR tracking and planes but no Depth API or LiDAR. Model estimates can have a different scale from the tracked floor: find each picture's own floor before scaling it to the measured floor's height.

Depth hit tests report `HitKind::EstimatedDepth` for model estimates and `HitKind::Depth` for measured depth, using the source of the picture the request actually hit. Recording and playback retain the source; existing version-one recordings still load, with depth treated conservatively as estimated because they did not record its source.

Apps that checked `frame.depth` as a boolean now use `frame.depth.is_some()` or match its source. Rebuild native hosts alongside their guests.
