---
"release": patch
---

# Recover rejected static releases

Fix the static release SSH command and enforce transfer limits while data arrives. Reject incomplete remote activation results, allow a corrected release after a candidate fails without changing live pointers, and verify restored routes before recording recovery. Enable the edge service for reboot and disable it when a failed initial release has no previous deployment. The static release, selected update and requested rollback path has real Linux/SSH acceptance.
