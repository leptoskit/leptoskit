---
"leptoskit_cloud": minor
---

# Store publishing needs the `publish` token scope

An API token with only the `deploy` scope can no longer publish to the app and extension stores. Publishing has its own permission now, which owners, admins and developers hold and which only the `publish` scope grants to a token, so that scope now lets a token publish where before it granted nothing but reading. A CI job that runs `lk cloud publish` needs both scopes, because the command uploads the source archive before it queues the publish: `lk cloud token create ci --scope deploy --scope publish`. A token with `deploy` alone is refused by `publish` with `403 insufficient_scope`. Reading a publish, its report and its log is unchanged.
