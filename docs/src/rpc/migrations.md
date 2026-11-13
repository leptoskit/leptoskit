# Migrations

RPC arguments and results are JSON through serde. There is no layout version and no migration climb on the wire.

Change a field with serde (`rename`, `default`, `alias`) so an older payload still deserializes. A result the caller cannot read is a client update, the same as any other JSON API.
