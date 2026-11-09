---
"leptoskit_testing": patch
"leptoskit_cli": patch
---

Linux pointer gestures from `leptoskit device` no longer pause for about fifteen seconds when the pointer is already on the point. `xdotool mousemove --sync` waits for a motion event that never arrives in that case, so a tap followed by a long press or a pan at the same point stalled. The move is sent without that wait.

Opening a Linux app also waits until one of its windows is visible, the process exits, or two seconds pass. Gestures used to be injected the moment the process was spawned, which is before the window is mapped, so the tap and the long press landed on the desktop.

Opening an Android app waits until that package is the focused window, then a little longer, or two seconds pass. `am start` was returning while the launcher was still focused, and the focus line itself updates before the activity accepts a tap, so the sanity tap and long press never reached the activity.

`leptoskit device boot` of an AVD that is already running fails immediately and names the emulator serial, instead of starting a second emulator and waiting out the boot deadline. If the emulator process exits before the device is online, the boot fails then, instead of blocking on `adb wait-for-device`.
