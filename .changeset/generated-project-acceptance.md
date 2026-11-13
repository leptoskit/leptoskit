---
"release": patch
---

# Require generated application journeys before publication

Publication now requires the database, auth and universal generated-app journeys on the validated release commit, before uploading artifacts or publishing crates. The same `devenv shell test:projects` command runs in PR/main CI and `test:all`. Missing tools, a missing individual test, and a failed journey stop the gate; Node-based client behavior cannot silently skip.
