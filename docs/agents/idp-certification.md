# OpenID certification for `leptoskit_idp`

What it takes to certify `leptoskit_idp` as an OpenID Provider, what blocks it today, and the order to do it in. The protocol notes are in [auth-plan.md](auth-plan.md#leptoskit_idp-sign-in-with-your-app); this file is only about certification.

## How certification works

OpenID certification is self-certification. The implementer runs the OpenID Foundation's conformance suite against a deployment, gets a passing result for each test plan of a profile, signs the Foundation's certification statement, pays its fee, and submits the plan results; the Foundation then lists the implementation. The suite runs either on the Foundation's hosted instance, which needs the provider reachable from the internet, or self-hosted from its source (Java, MongoDB, Docker Compose). Check the Foundation's certification pages for the current statement, fees, and submission address before starting: they change, so none is quoted here.

## Which profiles fit

| Profile                                            | Test plan                                           | Fits?                                                                                              |
| -------------------------------------------------- | --------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Basic OP                                           | `oidcc-basic-certification-test-plan`               | Runs nightly; one module fails by design (below).                                                  |
| Config OP                                          | `oidcc-config-certification-test-plan`              | Runs nightly and passes.                                                                           |
| Dynamic OP                                         | `oidcc-dynamic-certification-test-plan`             | Blocked: self-registered apps can't opt out of PKCE (below).                                       |
| RP-Initiated Logout OP                             | `oidcc-rp-initiated-logout-certification-test-plan` | Likely: `end_session` with `id_token_hint` and registered `post_logout_redirect_uri`s.             |
| Implicit OP, Hybrid OP                             |                                                     | No: neither flow exists, deliberately.                                                             |
| Form Post OP                                       |                                                     | No: `response_mode=form_post` isn't supported.                                                     |
| Session Management, Front- and Back-Channel Logout |                                                     | No: none is built.                                                                                 |
| FAPI 2.0 Security Profile                          | the suite's FAPI 2.0 Security Profile plan          | Closest fit for PAR + PKCE + DPoP + `private_key_jwt` + ES256; needs the gaps below checked first. |

The `oidcc-*` names are the suite's plan identifiers; confirm them, and find the FAPI 2.0 plan's, in the suite version used.

## What blocks it

1. **The no-nonce module.** The owner decided (IdP leftovers) that PKCE stays required by default, and that an operator may let a confidential app sign in without it, provided each such request asks for `openid` and carries a `nonce` (RFC 9700 §2.1.1). That lets the Basic and Config plans run, with the test server's apps opted out. But the Basic plan's `oidcc-ensure-request-without-nonce-succeeds-for-code-flow` sends neither PKCE nor a `nonce` and expects a code, so it fails by that rule; the nightly job lists it as expected. Certifying Basic OP needs either that module accepted as a failure by the Foundation, or the owner relaxing the `nonce` rule for opted-out apps. This is the owner's call.
2. **Dynamic OP.** Apps that register themselves can't opt out of PKCE, by the same decision, and the Dynamic plan registers its own clients and sends no PKCE. It stays out of reach unless the operator can pre-approve the opt-out for registrations (for example, with an initial access token that grants it).
3. **Manual review.** Four Basic modules end in `REVIEW`: `oidcc-prompt-login` and `oidcc-max-age-1` (a screenshot of the sign-in page shown again) and the two that expect the redirect-URI error page. The suite records the screenshots; a person checks them before submitting.

## How the nightly job runs

`examples/idp_conformance` is the test server: the provider over a fresh SQLite database, an HTML sign-in page with one button for a seeded person (through `leptoskit_auth`'s development sign-in, so a release build can't start), an HTML consent page, an RSA key for RS256 ID tokens, and three apps opted out of PKCE (`client`, `client2` and `client_secret_post`). Its `run-suite.sh` starts the suite's own Docker Compose with the test server reachable as `https://idp.test:9443`, and runs both plans with `expected-failures.json` and `expected-skips.json`; anything else that fails, warns, or is skipped fails the job, and so does an expected entry that stops happening. `.github/workflows/idp-conformance.yml` runs it nightly and on demand from a pinned tag of the suite. The same script runs locally with Docker or podman.

## What the provider doesn't do, which some modules try

Expect these modules to be skipped or to warn, and check each against the plan's pass criteria when it first runs: request objects (`request`, `request_uri`; discovery says both are unsupported), `display`, `ui_locales`, `claims_locales`, `acr_values` and the `acr` claim, `prompt=select_account`, `client_secret_jwt`, and `jwks_uri` for clients (dynamic registration takes inline `jwks` only).

For FAPI 2.0, check first: DPoP nonces (`DPoP-Nonce`) and binding the code to the key at the authorization endpoint (`dpop_jkt`) aren't built; `require_pushed_authorization_requests` can't be set per client; mutual-TLS client authentication and sender-constraining don't exist, so pick the `private_key_jwt` and DPoP variants.

## The order to do it in

1. ~~Decide the PKCE question.~~ Done: an operator opt-out with a required `nonce`.
2. ~~Build `examples/idp_conformance`, with an end-to-end test of its HTML forms.~~ Done.
3. ~~Add the nightly and `workflow_dispatch` conformance job.~~ Done, for Basic OP and Config OP. Add RP-Initiated Logout and FAPI 2.0 next, and fix what fails.
4. Settle the no-nonce module (above).
5. Deploy the test server at a public `https` address, run the same plans on the Foundation's hosted suite, review the screenshots, and export the results.
6. Sign the certification statement, pay the fee, and submit the results.
7. Keep the nightly job as the release gate for `leptoskit_idp`, as the plan says.
