---
"leptoskit_testing": patch
"leptoskit_cli": patch
---

# Connected iPhones show up in `device list`

`leptoskit device list` asked `devicectl` for `--json` and parsed a `devices` array at the top of that output. devicectl does not print JSON that way. It writes a file for `--json-output`, and the phones are under `result.devices`. The id an install, a launch, and an XCUITest session need is the hardware UDID, not the Core Device identifier. A plugged-in iPhone never appeared. Watches and simulators in the same file stay out of the iPhone list.
