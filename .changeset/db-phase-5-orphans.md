---
"release": minor
---

# Delete orphaned link targets, and shape link columns

`#[db::link(…, on_source_delete = delete_orphan_targets)]` deletes a link's targets once deleting their sources leaves nothing linking to them, in the same transaction: removing a post removes the tags no other post carries. A soft delete soft-deletes them and can be undone; a delete for good removes them only when no link at all is left. Each target goes through the database layer, so its hooks run and the audit log records why.

A shape can read a link's own columns: `#[db(via = tags_links)]` nests the link rows themselves, such as each tag's position on a post, with the tag nested inside. Links to rows the request can't see aren't there.
