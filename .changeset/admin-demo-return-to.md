---
"release": patch
---

# The admin demo's sign-in page stays on its own site

The `admin_demo` example's sign-in page keeps a `return_to` only when it's a path on the demo's own site. It used to keep a value such as `/\evil.example`, which a browser reads as `//evil.example`, so signing in from a crafted link could send the tab to another site. Such a value now sends the person to the site's front page, as a missing `return_to` always did.
