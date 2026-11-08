---
"release": patch
---

# Idle time on Windows

The power monitor plugin builds on Windows again. It read the time since the last keyboard or mouse input through the wrong `windows-sys` module, so apps that used it failed to compile for Windows.
