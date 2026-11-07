# Live action and stream output backpressure

The action concurrency semaphore was released before its result entered an unbounded completion channel. A session handling other work could therefore keep accepting new actions while completed replies accumulated. The completion channel now holds at most `MAX_ACTIONS` replies, and each reply retains its action permit until the session handles it. Running actions, queued completions, and the completion currently being handled share the same admission limit. A disconnected receiver releases retained permits.

Stream credits bound how many items a producer may read for the client. They did not bound queued server frames: a client can grant more credits before earlier items are delivered. Stream output now uses a bounded channel with the same capacity as the outbound connection queue. A producer awaits its send before reading another item. It can retain one item while waiting, so the bound includes both queued frames and at most one waiting frame per active stream. Idle streams reserve no output capacity. Terminal failures also await capacity, rather than disappearing when the queue is full.

Stream frames larger than `MAX_FRAME_BYTES` produce an explicit 413 terminal failure and stop the producer. `StreamSink::fail` retains its synchronous signature so existing generated pumps, the invalid-argument path, remote-service stubs, and handwritten pumps remain compatible. It records the first terminal error. The session awaits the sink's completion after the pump returns, before the tracked producer task finishes. The bounded send never becomes a detached task. Macro source and snapshots remain unchanged.

Flushing already-produced stream frames during drain or reauthentication uses the connection's stall timeout too. Failure stops the session rather than waiting indefinitely on a raw channel send or continuing to announce successful reauthentication.

## Acceptance

Unit tests exercise action permit retention through queueing and consumption, closed receivers, stream credit replenishment under backpressure, producer cancellation, retained terminal errors, oversized frames, and a ready stream sharing output with an idle one. The parent probe compiles the actual stream sink implementation, completion structure, and wire protocol with cached Tokio, futures, serde, and JSON dependencies. Only the outer operation error type and resumable trait boundary are stubbed. This is not a full RPC crate, service macro, database, or network acceptance test.

Integrate with PR #611 and the bounded hub mailbox before full acceptance. Run the actual crate tests and macro snapshots, both database dialects, stream cancellation and reauthentication tests, service stream generation, the live session suite, and mandatory generated SDK journeys. Retain the existing cursor, slow-reader, identity, and connection-limit assertions. Run normal local and push gates.

Further review remains necessary for buffered frames when a stream ID is cancelled and reused, client command and subscriber queues, and memory limits on arbitrary application-produced values. These changes bound the named server queues; they do not establish a process-wide memory bound or guarantee cancellation of application-side work.
