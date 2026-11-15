---
"leptoskit_cli": minor
---

# `lk admin serve --analytics-url`

`lk admin serve` connects the admin's Analytics area to your site's analytics service. It takes the address from `--analytics-url`, or from `LEPTOSKIT_SERVICE_ANALYTICS_URL` when the flag isn't given, and the token from `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`. If an address is given but the token is missing, it stops and says which variable to set. With neither, the Analytics pages explain how to connect.
