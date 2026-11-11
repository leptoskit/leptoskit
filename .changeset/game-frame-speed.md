---
"release": patch
---

# Faster game frames

The game host keeps one wgpu device for the whole run, stamps HUD text onto the picture instead of drawing one GPU shape per pixel, and fills CPU rows in spans. A missing adapter is remembered. The run report includes `frame_us`, the mean time of a frame.
