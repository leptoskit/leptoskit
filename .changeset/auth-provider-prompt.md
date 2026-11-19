---
"leptoskit_auth": minor
"leptoskit": minor
---

# Ask a provider to sign in again

`StartProvider` has a `prompt` field. `Some(Prompt::Login)` asks the provider to sign the person in again even when someone is already signed in there, which is how a person switches to another account. `Some(Prompt::SelectAccount)` shows the provider's account picker. The value reaches the provider's authorization page as OpenID Connect's `prompt` parameter, and a provider that doesn't know the parameter ignores it.

```rust
sign_in.mutate(StartProvider {
	provider: "main".into(),
	redirect_to: Some("/admin/sign-in".into()),
	prompt: Some(Prompt::Login),
});
```

Code that builds a `StartProvider` needs `prompt: None` to keep today's behaviour.
