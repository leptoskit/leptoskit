---
"leptoskit_cloud": minor
---

# Only people manage members and tokens in LeptosKit Cloud

An API token can no longer invite people, revoke invitations, change a member's role, remove a member, or list, make or revoke tokens, whatever its scopes: those answer `403 person_required` to a token, and need someone signed in, in the browser or through the CLI. The `admin` scope now means deleting projects and environments, and nothing more. A leaked token can't turn itself into a membership or a longer-lived token.

`create_token` is now an action, so a token's value is never stored anywhere, not even as the remembered answer to a request with an `Idempotency-Key`; the header is ignored. Instead a live token's name is unique in its organisation, and asking again for a name a live token has answers `409 token_name_taken`, naming that token so it can be revoked, rather than quietly making a second one.

An invitation is only as good as its sender's authority when it is accepted: if they were demoted or left, it no longer works, and their pending invitations they can no longer give are revoked. Inviting an address again needs the authority to cancel each invitation it replaces, and each one cancelled is written to the audit log.

A personal organisation's name falls back to a random suffix after `ana`, `ana-2` and `ana-3`, so nobody can stop someone's first sign-in by taking every name their address suggests.
