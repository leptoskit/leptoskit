# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-21)

### Features

- **Make `Keys::seal` and `Keys::open` public.** Another crate can now seal bytes under a set of rotating keys, bound to the context parts it names, the way sessions and codes already are. `Keys::shares_key_with` says whether two sets hold the same key under any name, so a server can refuse to use one key for two jobs. _Owner:_ Ifiok Jr. · _Introduced in:_ [25cddbe](https://github.com/leptoskit/leptoskit/commit/25cddbed387b5b2483808d0171a5b063884ba739)

#### Ask a provider to sign in again

_Owner:_ Ifiok Jr. · _Introduced in:_ [f25ea8c](https://github.com/leptoskit/leptoskit/commit/f25ea8c94f5875355ce081afae5ac92a652cee48)

`StartProvider` has a `prompt` field. `Some(Prompt::Login)` asks the provider to sign the person in again even when someone is already signed in there, which is how a person switches to another account. `Some(Prompt::SelectAccount)` shows the provider's account picker. The value reaches the provider's authorization page as OpenID Connect's `prompt` parameter, and a provider that doesn't know the parameter ignores it.

```rust
sign_in.mutate(StartProvider {
	provider: "main".into(),
	redirect_to: Some("/admin/sign-in".into()),
	prompt: Some(Prompt::Login),
});
```

Code that builds a `StartProvider` needs `prompt: None` to keep today's behaviour.

#### Control-plane scaling, HA sweeper, CA TLS, and billing

_Owner:_ Ifiok Jr. · _Introduced in:_ [713fddf](https://github.com/leptoskit/leptoskit/commit/713fddf6095efdf17fbbbbd7e6ea87d9c2d586a6)

- `leptoskit_db` now supports custom Postgres CA PEM bundles via `sslrootcert=` or `PGSSLROOTCERT`, and adds session-scoped `Db::try_advisory_lock` for HA leader coordination.
- `update_environment` enforces plan scale limits (`max_size`, `max_replicas`, `require_sleep`, also exposed on `PlanLimits` and enforced on `rollback`) and queues an immediate live-scale cluster release when the caller holds `Permission::Deploy` (allowing scale-down even over spend cap, skipping no-op rollouts, honoring cluster facts, and preserving `schema_uncertain`). Forward deploys clear `scale_differs` once applied.
- Added `cluster_environments` worker RPC so restarted cluster workers immediately re-hydrate tracked environments for status, logs, and compute metering.
- `ShippingSettings` gains O(1)-tracked `organization_runtime_log_cap` (via `log_usage.runtime_bytes`), `run_sweeper` (`LEPTOSKIT_CLOUD_SWEEPER`), and Postgres advisory-lock sweeper leader election across HA replicas.
- Hardened `S3Store` (disabled redirects, bounded streaming hash concurrency, `400`/`403`/`501` checksum fallback, content-length check), `Stripe` (sanitized error telemetry and key length check), `subscriptions` (concurrent checkout customer conflict handling and overdue sweep rotation), and `bootstrap_operator` (concurrency-safe, checks disabled accounts, only runs when no operators exist).
- `leptoskit_cloud_app` adds a `production = ["ssr", "postgres"]` feature flag.

### Fixes

- **Count a request by its connection without the header.** With `client_ip_header` set, a request the header was missing from, or whose header wasn't an address, had no address at all, and the limits kept per address didn't count it. Such a request is now taken to come from its connection's address. _Owner:_ Ifiok Jr. · _Introduced in:_ [373ad29](https://github.com/leptoskit/leptoskit/commit/373ad295394a26e69e883a6b28983d35b7e3f02d)
- **Native sign-out clears the session query when offline.** `sign_out_mutation()` now clears a native app's session query when it forgets its local tokens, including when the server is unreachable or refuses the request. The mutation still reports that error: local sign-out does not claim the server revoked the session. A browser keeps its session query on failure because its cookie remains until the server removes it. _Owner:_ Ifiok Jr. · _Introduced in:_ [10a1d5d](https://github.com/leptoskit/leptoskit/commit/10a1d5dbcfd61ecb6eaefa14e0a55670a5680dcd)

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_auth: sign-in and sessions

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_auth` is sign-in for leptoskit apps, stored in the app's own database: the `auth` schema pack, revocable sessions, email codes, authenticator apps with recovery codes and step-up, passkeys, third-party providers, opt-in passwords, Solana wallets, account merging, and the signed-in user that access policies read.

Enable `auth` on `leptoskit`, plus `auth-passkeys`, `auth-providers`, `auth-passwords`, `auth-solana`, `auth-smtp`, or `auth-webhook` in the server's crate for the parts that need them. The headless client works on the web and in the generated iOS, Android, and macOS apps, where it uses the passkeys, web-auth, secure-store, and HTTP plugins. `leptoskit_idp` turns the same accounts into an OIDC provider.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
