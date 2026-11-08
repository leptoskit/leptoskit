---
leptoskit_plugin_image_picker: minor
---

# Library image and video picker

`leptoskit_plugin_image_picker` picks one photo-library image or video with cancellation, an exclusive busy slot, and a bounded temporary URI plus width/height/mime/size metadata. Camera capture stays for a later `camera` plugin. Apple uses PHPicker; Android uses the activity-result broker; the browser returns a `blob:` URI from a trusted file input.
