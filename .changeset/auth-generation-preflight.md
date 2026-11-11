---
"release": patch
---

# Check auth module integration before writing files

`lk generate auth` parses client and server roots before generation. Existing module declarations with comments, unusual spacing, or visibility no longer receive a duplicate declaration. Inline modules, redirected module paths, duplicate declarations, and a conflicting server module directory fail before screens or configuration are written, even with `--force`.

Setup reminders recognize direct function calls instead of comments, strings, or function references. Providing auth without gating screens keeps the root integration reminder. These reminders do not verify startup behavior, routing, or database authorization; existing apps still need the printed integration steps and runtime checks.
