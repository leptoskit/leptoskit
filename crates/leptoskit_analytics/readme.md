# leptoskit_analytics

Product analytics for LeptosKit apps.

The binary endpoint accepts at most 4 MiB per request and 16 MiB after gzip expansion. Decoding also has a 32 MiB allocation budget: dictionary references, repeated identities and collection reservations count toward it. A small encoded batch cannot expand an interned name or identity into unbounded allocations. Malformed or over-budget batches answer `400`; no event from that batch is queued. Extreme client clock corrections are clamped to the receive time.
