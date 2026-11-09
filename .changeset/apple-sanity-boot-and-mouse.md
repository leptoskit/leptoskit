---
"leptoskit_testing": patch
"leptoskit_cli": patch
---

# iOS sanity boots a shut-down simulator, and macOS clicks with a mouse

`leptoskit device sanity` on an iOS simulator now boots that simulator before `simctl install`. A shut-down simulator refuses the install. One that is already booted stays up: the boot step treats that state as success. A sanity run that does not install, and a run on a real iPhone, still do not boot.

Pointer actions sent to a macOS session use a mouse. Mac2 does not accept a touch pointer. iOS, Android, Linux, Windows, and the web still send touch.
