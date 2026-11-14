---
"release": minor
---

# An admin app can sign people in through your site's own sign-in

`leptoskit_idp` gains a standard `roles` scope: when your app registers a `Roles` hook, one of your own first-party apps may ask for it, and the person's roles from that hook land in its ID token and `userinfo` as `"roles": ["admin", …]`. Another site's apps are refused the scope at authorize, and an app acting as itself can't have it.

`leptoskit_auth` closes the loop for the app that signs in through yours:

- `Auth::roles(account)` reads the registered `Roles` hook, `auth::require_role("admin")` guards a leptos server function, `auth::WithRole<Admin>` is the axum extractor, and `Auth::require_role(&me, "admin")` the check behind them.
- `Provider::userinfo(true)` (and `[auth.providers.<id>] userinfo = true`) fetches `/userinfo` with the access token after the code exchange, checks its `sub` against the ID token's, and makes the merged claims available to a gate.
- `AuthBuilder::gate(AdminGate)` registers a `SignInGate`, which runs once per sign-in — through a provider, an emailed code, a password, a passkey, or a wallet — before any session or account is made, so a refusal creates nothing and the sign-in page sees the `not_allowed` error code. `SignInAttempt` names the method, the provider's subject, and the provider's claims, with `roles()` reading the `roles` claim.
- `[auth] methods = ["oidc:main"]` (`AuthBuilder::sign_in_methods`) turns every other way to sign in off: its routes answer 404 and the sign-in pages see none of it. The default is unchanged.

See "Use your site's sign-in for an admin app" in the sign-in-with-your-app guide.
