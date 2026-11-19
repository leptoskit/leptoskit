---
"leptoskit_cloud": minor
---

# Sell plans through Stripe

`leptoskit_cloud::billing` adds `subscription`, `checkout` and `portal`. With a Stripe key, a webhook secret and a price ID for each plan that is sold, an organisation's owners and billing members can subscribe to a plan on Stripe Checkout and manage it on Stripe's customer portal. The plan changes only when Stripe says the subscription did: the control plane verifies each event's signature at `/webhooks/stripe`, then asks Stripe what the subscription is now.

`ControlConfig` gains `payments`, so code that builds one by hand must set it (`None` sells nothing). The `leptoskit_cloud_app` server reads `LEPTOSKIT_CLOUD_STRIPE_KEY`, `LEPTOSKIT_CLOUD_STRIPE_WEBHOOK_SECRET`, `LEPTOSKIT_CLOUD_STRIPE_PRICES`, `LEPTOSKIT_CLOUD_STRIPE_AUTOMATIC_TAX` and `LEPTOSKIT_CLOUD_STRIPE_TEST_MODE`; a Stripe test key is refused outside `--dev` without the last. A plan that is sold needs a `monthly` price and a `spend_cap` in the catalogue, and `set_spend_cap` now refuses a cap above the plan's own (`cap_above_plan`). Usage beyond what a plan includes is still not invoiced.
