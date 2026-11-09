---
"leptoskit_testing": patch
"leptoskit_cli": patch
---

`leptoskit device open` fails when the app never takes the screen, instead of reporting success while the launcher is still in front. A tap, long press, or pan past the display fails too: `input` exits 0 for those points and delivers nothing. An Android pinch still waits for its touchscreen to attach, but that wait is no longer part of the gesture clock, so a 300 ms pinch is 300 ms of contact with both pointers. `device list` marks an AVD as running when that emulator is already up. `leptoskit mcp http` answers `application/json`, and the screenshot tool's schema resolves.
