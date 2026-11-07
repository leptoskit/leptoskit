# Live hub mailbox

The hub used an unbounded channel of database-round batches for each connection. A session can pause while it runs a query, checks identity, handles an action, or waits for outbound capacity. Meanwhile, the shared hub continues producing batches and retaining old results for that session.

The mailbox retains one batch. When another round arrives before the session consumes it, the mailbox combines the changed subscription IDs and keeps the newest result for each. It converts patches to their complete resulting values, because a patch can depend on an intermediate value that coalescing removes. Errors replace successes and remain present until a later success. Identity revalidation and full-resynchronization flags survive coalescing. The combined batch reaches the newest database version represented by its results.

There are at most 256 active subscriptions per session. Leaving a group removes its ID from the pending mailbox, including before a replacement subscription registers the same ID. An impossible oversized batch closes the mailbox without delivering a partial result or advancing the cursor. The session treats mailbox closure as connection failure. The hub never waits for a session to consume its mailbox.

This bounds retained hub batches and their subscription membership. It does not bound total process memory independently of connection count or query-result size. Group results, pending outbound transitions, action completions, stream frames, and client-side subscriber queues have their own ownership and limits. These are separate acceptance concerns.

## Acceptance

Mailbox unit tests cover sustained writes, complete-value recovery after patches, latest errors, identity invalidation, releasing replaced values, subscription removal and ID reuse, membership limits, receiver cancellation, and closing either endpoint. The standalone parent probe compiles the production mailbox with real Tokio and JSON dependencies, the actual `Batch` and `Changed` definitions, and small `Patch` and `WireError` protocol stubs. Its ten passing tests establish mailbox behavior only; they do not type-check the entire RPC crate or exercise database delivery.

Before merging, integrate with the live session recovery changes in PR #611 and execute the actual RPC crate tests, including both database dialects, authentication changes, slow-reader recovery, split transitions, and all generated SDK journeys. Add a session-level acceptance case that holds a session while multiple database rounds complete, then verifies the latest state and error without requiring another write. Recheck replacement subscriptions and unchanged neighboring sessions. Preserve the old slow-reader and reconnect assertions. Run the normal full local and push gates.
