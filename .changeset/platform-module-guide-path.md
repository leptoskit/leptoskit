---
"leptoskit": patch
---

Fix the platform module guide's path from the source root

The creating-an-app guide now uses `components/heading` for a module in `src/components/heading.rs`, matching `platform_module!`'s source-root-relative paths. The generated application's end-to-end test uses the same corrected declaration and still verifies adding and removing a platform override.
