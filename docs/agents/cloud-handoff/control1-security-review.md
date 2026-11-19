# Security review of Control 1 (accounts, organisations, tokens, projects)

An independent reviewer traced these through the source of the control plane's first slice. File paths are relative to the repository; line numbers are from the reviewed commit and may have shifted slightly.

## Findings

### 1. High: API tokens can escape their scope and expiry through invitations

Location: `crates/leptoskit_cloud/src/server/account.rs` around lines 307 and 497.

Sequence: an owner creates an organisation-wide token with only `["admin"]`, expiring in 30 days. Its holder cannot call `create_project` or mint a token with `manage`. The holder calls `invite_member(organization, attacker_email, owner)`, signs in separately with that verified address, and calls `accept_invitation(link_token)`. Acceptance creates a permanent owner membership. That account can create projects and mint 365-day tokens with additional scopes. Revoking or expiring the original token does not remove this membership.

Consequence: a scoped credential becomes unrestricted, persistent tenant access. An admin-created token can similarly bootstrap a permanent admin account. Acceptance also does not recheck the inviting credential or the inviter's current authority.

Reviewer's smallest fix: require a person's session for invitations and role changes that grant membership authority; invalidate outstanding token-created invitations.

### 2. High: idempotency receipts store complete API token secrets in plaintext

Location: `crates/leptoskit_cloud/src/tokens.rs` around line 142; supporting path `crates/leptoskit_rpc/src/server/run.rs` around line 326.

Sequence: an authorised caller sends `POST /rpc/create_token` with an `Idempotency-Key`. The operation returns `CreatedToken`, including its complete `lkc_…` value. The RPC mutation runner serializes that result into `rpc.receipts.result`. Repeating the same request returns the stored secret.

Consequence: a database or backup reader can recover usable credentials without attacking their hashes. "Hashed at rest" and "shown once" fail even though `api_tokens.secret_hash` itself is correct.

Reviewer's smallest fix: disable receipt persistence for this secret-returning operation; until that is supported explicitly, reject its idempotency keys before receipt dispatch.

### 3. Medium: one customer can prevent other customers from completing onboarding

Location: `crates/leptoskit_cloud/src/server/personal.rs` around line 52.

Sequence: a signed-in attacker calls `create_organization` to occupy `personal`, `personal-2`, …, `personal-50`. A new wallet-only account then calls `whoami` or any other operation. Identity resolution tries exactly those 50 candidates for its personal organisation and returns a server error; repeating the call cannot recover. Occupying the candidates for a known email local part targets an individual sign-up.

Consequence: about 50 ordinary requests block cloud access for every new account that uses the default `personal` base. Identity creation runs before the requested operation, so affected users cannot reach another operation to resolve it.

Reviewer's smallest fix: after friendly-name collisions, use a sufficiently random suffix instead of exhausting a predictable, finite namespace.

### 4. Medium: admins can revoke owner invitations through the replacement path

Location: `crates/leptoskit_cloud/src/server/account.rs` around line 313.

Sequence: an owner invites `recipient@example.com` as `owner`. An admin calls `invite_member` for the same address with `role = viewer`. Authorisation checks only the newly requested viewer role. The replacement revokes every pending invitation for that address, including the owner invitation.

Consequence: an admin can cancel an owner's invitation although `revoke_invitation` requires owner authority for that same action. The replacement also writes no audit event naming the revoked invitation.

Reviewer's smallest fix: read the invitations being replaced and apply the role check to their existing roles before revoking them.

## Test coverage gaps the reviewer named

- The authorisation matrix's inventory check (`crates/leptoskit_cloud/tests/authorization.rs` around line 37) ignores operations without one of three recognised tags, so a new untagged operation can evade it.
- Scope tests use owner-created tokens rather than the full role × scope × project-binding matrix.
- Cross-tenant scenarios cover selected operations, not every matrix cell.
- No test covers: receipt-secret persistence, invitation-based token escalation, owner-invitation replacement, exhausted personal slugs, concurrent last-owner changes, concurrent cap enforcement, or live-socket permission changes.

## Other observations

- The audit log is append-only by application convention only; nothing in the database enforces it.
- There is no rate limiting on control operations.

## Checked and found sound

Direct tenant access (organisation resolved from the database; raw member, invitation and token IDs constrained to it; project-bound tokens checked against the resolved project). Direct token minting (scope subsets, creator-role restriction, parent expiry; OS randomness, SHA-256, constant-time comparison, uniquely indexed prefixes). Concurrency (serializable transactions; uniqueness constraints in both dialects). CLI authorisation (the identity provider checks the `cloud` scope; cloud identity requires the registered CLI client ID). The RPC macro change (a correct loosening: delegated tokens carrying all required scopes are now allowed on annotated operations; unannotated operations remain denied). Inputs and server configuration (typed predicates; names reject control characters; plain-text mail; production requires keys and an HTTPS origin; `--dev` is loopback-only and refused in release builds; 2 MiB RPC bodies).
