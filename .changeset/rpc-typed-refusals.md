---
"leptoskit_rpc": minor
"leptoskit_rpc_codegen": minor
---

# Let an operation refuse with a code and a wait

A query, mutation, or stream can now say why it refused, in a form a screen can match on without comparing sentences. `rpc::Error::rate_limited(wait)` answers `429` with a `Retry-After` header, and `rpc::Error::refused("notes_full", "Your notebook is full")` answers `409` with a code the app chose. Build `rpc::Error::Refused { code, message, status }` for another status.

The caller gets the new `FnError::Refused { status, code, message, retry_after }`, with `FnError::code()` and `FnError::retry_after()` as shortcuts. It arrives the same way over a plain call, the live socket, and a stream, on every platform. A refusal is never retried.

The generated Rust, TypeScript, Dart, Go, and Python clients read it too: each gains a `refused` kind with the code and the wait, and each `LiveError` carries them over the live connection.

Things to know when upgrading:

- `leptoskit_rpc::Error` and `FnError` each gained variants, and `leptoskit_rpc::live::WireError` gained the optional fields `code` and `retry_after`. Code that matches on them without a wildcard arm, or builds a `WireError` by hand, needs updating.
- The generated Rust client's `Error` gained a `Refused` variant, and its `LiveError` two fields.
- A client built or generated before this release reads a refusal as an `other` error whose text is the raw `Refused|{…}` body. Release clients before the server starts refusing this way.
