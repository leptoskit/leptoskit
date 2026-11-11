---
"leptoskit_cli": patch
---

Allow protected systemd drop-in masks that point exactly to /dev/null during graph release and recovery. Active overrides and masks with unsafe ownership or parents remain rejected.
