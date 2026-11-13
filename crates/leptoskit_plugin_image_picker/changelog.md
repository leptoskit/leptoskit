# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-13)

### Features

- **Library image and video picker.** `leptoskit_plugin_image_picker` picks one photo-library image or video with cancellation, an exclusive busy slot, and a bounded temporary URI plus width/height/mime/size metadata. Camera capture stays for a later `camera` plugin. Apple uses PHPicker; Android uses the activity-result broker; the browser returns a `blob:` URI from a trusted file input. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #674](https://github.com/leptoskit/leptoskit/pull/674)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-08)

### Features

#### leptoskit_plugin_image_picker: library image and video pick

`leptoskit_plugin_image_picker` is the leptoskit plugin for picking one photo-library image or video with cancellation, an exclusive busy slot, and a bounded temporary URI plus width/height/mime/size metadata. Camera capture is deferred to a later `camera` plugin. Apple uses PHPicker (app-owned purpose strings remain for any future limited-library APIs); Android uses the activity-result broker; the browser uses a trusted file input and a `blob:` URI.
