# Live recovery boundaries

A session keeps one unfinished transition. Later results coalesce into a full snapshot, retaining errors as well as successes. Capacity wakes the writer without another database write or client frame. A stalled transport closes at its configured timeout. Randomized selection and bounded batch draining let client input compete with a continuously busy database.

Prefix frames retain the previous committed cursor; only the final frame advances it. Welcome reports server state but does not advance the applied client cursor. Reconnecting between chunks therefore requests recovery from the last complete transition.

This bounds the session recovery backlog by subscription state plus one transition. It does not bound total memory: the existing hub-to-session batch channel is unbounded, values have application-dependent sizes, and stream/action queues are separate. This change does not redesign those queues.

## Terminal producer cleanup

Consuming the current generation's terminal frame aborts its tracked producer before retiring the stream. A handwritten producer can send its terminal through the sink and then keep awaiting other work; dropping its task handle alone would detach that work from session cleanup. Stale terminal frames still cannot cancel a replacement generation.

The regression test inserts a running producer with a destruction signal, delivers its terminal, and requires the producer to be dropped. It fails on the previous implementation and passes with cancellation. The 114 RPC library tests pass with the fix. Mini01 retains the negative and positive logs in `.git/queue-evidence/parent-terminal-{negative,positive}.log`. Required PostgreSQL checks use the validated PostgreSQL binary directory on both `LEPTOSKIT_POSTGRES_BIN` and `PATH`, including shell-based deployment and recovery fixtures.
