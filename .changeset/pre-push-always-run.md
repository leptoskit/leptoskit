---
leptoskit_cli: patch
---

Always run the repository's full pre-push gate, including pushes whose commits change no files. The gate no longer depends on the hook runner selecting at least one changed file.
