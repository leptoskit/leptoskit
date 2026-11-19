---
"leptoskit_notifications": minor
---

# Import the device tokens you already hold

An app that moves from Firebase Cloud Messaging has a table of device tokens, and until now none of them could be used: every device had to update and enroll before it could be pushed to. `push::import_push_endpoints` takes those tokens from your app's server, so people are pushed to from the next send on.

- The call takes 1 to 500 entries (`MAX_IMPORT_ENDPOINTS`), each an `ImportEndpoint`: a person, an FCM registration token or an APNs device token, and optional `DeviceDetails`. A Web Push subscription cannot be imported.
- It answers `ImportedEndpoints`: how many were `imported`, how many were `unchanged` because their person already holds the token, and the `skipped` entries by index, each `held_by_another` or `person_full`. No credential is answered.
- One malformed entry refuses the whole call and names its index. Nothing is written.
- An import never moves a token between people, and never revives an endpoint a provider invalidated.
- `EndpointSummary` has a new `imported` field. It is `true` until the device's own app enrolls the same token for the same person. The endpoint then keeps its id and its queued pushes and takes the details the device reports.

Only your app's server may call it.

**What changes for an existing app:**

- The `endpoint_import` migration adds one nullable column, `push_endpoints.imported_at`, on SQLite and Postgres. Existing rows are not changed.
- `EndpointSummary` has a new public field. Code that builds one by naming every field no longer compiles. The field is left out of the JSON when it is `false`, so existing clients read the same answers as before.
- The service keeps no record of a revoke or an erasure. Importing a token again after its person turned notifications off, or after `forget_person`, enrolls it again.
