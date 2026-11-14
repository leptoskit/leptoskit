---
"leptoskit_cli": minor
---

# `lk admin serve`

`lk admin serve --project <repo> --url <admin origin> --main-site <site origin> --client-id <id>` runs the admin for the repositories you name, with the deployment service and its worker inside it. It reads the client secret your site issued from `LEPTOSKIT_ADMIN_CLIENT_SECRET`, its session keys from `LEPTOSKIT_ADMIN_KEYS`, and the deployment service's owner credential from `LEPTOSKIT_CONTROL_KEY`. `--role` changes which role gets in, and `--environment` labels a staging admin.
