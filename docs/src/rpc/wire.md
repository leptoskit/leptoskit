# RPC wire

Operations travel as JSON through serde.

- **Content-Type:** `application/json`
- **HTTP entrypoint:** `POST /rpc/<op>` (override with `[rpc].path`)
- **Arguments** are one JSON object, the generated struct for the function. Field attributes `#[serde(...)]` and `#[schemars(...)]` on the function parameters land on that struct. `schemars::JsonSchema` is the validation and description library for those arguments.
- **Results** are the `T` in `rpc::Result<T>`, encoded with serde. Annotate that type with `Serialize`, `Deserialize`, and whatever serde attributes it needs. It does not need a second codec.

A query or mutation can take its context as the first argument, or omit it and read the same value from the task:

```rust
#[rpc::query]
pub async fn notes(starting: Option<String>) -> rpc::Result<Vec<String>> {
    let cx = rpc::cx()?;
    let notes = Note::query().all(cx.db()).await?;
    Ok(notes.into_iter().map(|note| note.title).collect())
}
```

`rpc::cx()` is the query. `rpc::mutation_cx()` is the mutation. Hold the returned `Arc` for as long as you use it. Passing `cx: &rpc::QueryCx` still works, and nested `rpc::call` uses that reference.

`[rpc].transports` and `[rpc].default_transport` are `json`. That is the only transport. A request with another content type is rejected. To change a field without breaking an older payload, see [Changing a JSON field](migrations.md).
