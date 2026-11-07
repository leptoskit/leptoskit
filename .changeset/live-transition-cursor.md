---
"release": patch
---

# Finish split updates before advancing the resume cursor

A live transition that needs several frames now advances its version only on the final frame. Reconnecting after an earlier frame replays the unfinished changes, and mutations continue waiting until the complete transition arrives. The frame format is unchanged.
