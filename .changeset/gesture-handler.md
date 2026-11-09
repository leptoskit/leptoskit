---
"leptoskit_native": minor
"leptoskit_android": minor
"leptoskit_gtk": minor
"leptoskit": minor
---

# Gestures on a view

A view can follow a finger. `gesture(Gesture::tap())`, `pan`, `long_press`, `pinch`, `rotation`, and `fling` recognize the pointer in one arena shared by the Android widget backend and Linux. `simultaneous`, `exclusive`, and `race` compose recognizers the way a pinch and a rotation share one gesture, or a pan wins over a tap. A drag that wins claims the pointer so the control underneath does not also click. A tap does not, so a button still presses. iOS, macOS, Windows, the web, the terminal, and the generated Swift and Kotlin renderers still deliver presses and clicks only.
