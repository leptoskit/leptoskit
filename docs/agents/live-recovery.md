# Live recovery boundaries

A session keeps one unfinished transition. Later results coalesce into a full snapshot, retaining errors as well as successes. Capacity wakes the writer without another database write or client frame. A stalled transport closes at its configured timeout. Randomized selection and bounded batch draining let client input compete with a continuously busy database.

Prefix frames retain the previous committed cursor; only the final frame advances it. Welcome reports server state but does not advance the applied client cursor. Reconnecting between chunks therefore requests recovery from the last complete transition.

This bounds the session recovery backlog by subscription state plus one transition. It does not bound total memory: the existing hub-to-session batch channel is unbounded, values have application-dependent sizes, and stream/action queues are separate. This change does not redesign those queues.
