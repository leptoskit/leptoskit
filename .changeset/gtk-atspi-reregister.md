---
"leptoskit_gtk": patch
---

On GTK 4.22, buttons, radios, switches, checkboxes, and drawings are registered with assistive technologies after the window is on screen. That release could assign them a bus path before the accessibility connection was up and then never publish the object, so a screen reader saw the frame and the slider but not the named control.

Right-to-left text fields on that release also keep logical start on the right. GTK 4.22 no longer mirrors `xalign` itself, so the older compensation was pinning the text to the left.
