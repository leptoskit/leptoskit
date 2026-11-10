---
"release": minor
---

# Rotate the provider's RSA key without breaking apps

`leptoskit::idp` can now take a next RSA key ahead of time: `next_rsa_key_env` in `[auth.idp]`, or `IdpBuilder::next_rsa_key`.

- **Published first:** the next key appears in the JWKS at once, so apps that keep the key set have it before any token needs it. It starts signing a day after it was first published (`Lifetimes::next_key_notice`), on every server together.
- **The old key goes quietly:** it stops signing at the switch and leaves the key set once the last token it signed has expired. Moving the new key into `rsa_key_env` afterwards changes nothing for apps.
- **Leaks:** a key replaced outright leaves the key set an hour or so after the server stops having it, where it used to stay published for up to a signing period.
- **JWKS publishes the keys that sign next** before the first token, so an app that reads the key set before anyone signs in can check the first token too.
