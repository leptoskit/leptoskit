---
"leptoskit_gtk": patch
---

Text fields on GTK 4.18 and later follow the direction the layout resolved. The entry's inner text does not inherit that direction, and those releases reverse alignment from the widget rather than from the first strong character, so Arabic and Latin in a right-to-left field both sat on the left.
