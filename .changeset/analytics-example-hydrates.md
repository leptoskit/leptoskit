---
"release": patch
---

# The analytics example's page responds again

The analytics example's buttons did nothing, and its counts never updated, because the page stopped while starting in the browser. It now starts the browser's task runner before connecting to the analytics service, so the Buy and Share buttons record events and the summary updates live. Each event's row now shows its new count when it changes, instead of the first count it showed.
