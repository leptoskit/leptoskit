---
"release": minor
---

# Dart clients keep requiredness and nullability separate

Generated Dart models and operation arguments no longer treat a nullable type as an optional one. A required nullable field or argument stays required and always writes its key, including an explicit `null`. Optional nullable values use the new three-state `Presence<T?>` wrapper: `const Presence<T?>.absent()` omits the key, `Presence<T?>.present(null)` sends `null`, and `Presence<T?>.present(value)` sends the value. Optional non-nullable values keep their `T?` shape, so `false`, `0`, empty strings, empty lists, and empty maps are still sent instead of dropped.

JSON and live payloads share the same presence-aware argument map, so HTTP calls, watched queries, live mutations, and channel join arguments round-trip missing, `null`, and real values. Query-string and form encodings cannot represent explicit `null`; `--strict` now reports that remaining encoding limitation instead of claiming the nullable field was lossless.

Regenerating changes the Dart signatures of optional nullable fields and arguments from `T?` to `Presence<T?>`, and required nullable constructor arguments become `required`. Update callers to pass `Presence.present(...)` or `Presence.absent()`, and read `.isPresent`/`.value` when consuming an optional nullable field.

The mandatory SDK acceptance gate now analyzes and runs generated Dart presence fixtures alongside every existing SDK journey. Release binary builds explicitly retain locked dependency resolution.
