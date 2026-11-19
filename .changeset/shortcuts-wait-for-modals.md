---
"leptoskit_native": patch
---

# App shortcuts wait while a modal is open

On the web, an app shortcut such as Escape or W no longer runs while a `Modal` is open, and it no longer takes the key from the modal. Escape now closes the modal, as it should. Before, an app with its own Escape shortcut kept the key, so the modal stayed open. Shortcuts work again as soon as the modal closes.
