---
"release": patch
---

# Live recovery retains current query errors

When a live query fails, a full recovery transition now repeats that failure instead of replacing it with an earlier successful value. Reauthentication failures also replace the stored result before any recovery snapshot is produced. A subsequent successful query restores the current value.
