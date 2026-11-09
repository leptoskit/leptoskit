---
"release": patch
---

# Preserve unrelated routes during rollback

Give every graph release transaction its own edge configuration so rolling back one service preserves other services' current bindings. Keep earlier transaction history and recheck preserved routes before promotion. Reject mismatched worker request contexts before cleanup can cancel an existing prepared release.
