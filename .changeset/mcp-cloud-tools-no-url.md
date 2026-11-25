---
"leptoskit_cli": patch
---

# Cloud tools on `lk mcp` take no `url`

The `lk mcp` cloud tools (`cloud_estimate`, `cloud_deploy`, `cloud_status`, `cloud_logs`, `cloud_secrets_list`) take no `url` argument. It came from the model or the client, and with `LEPTOSKIT_CLOUD_TOKEN` set one tool call could send that token to any origin the argument named. The tools now choose the cloud as `lk cloud` does without `--url`: `LEPTOSKIT_CLOUD_URL`, then `[cloud].url` in `leptoskit.toml`, then the hosted cloud. The token still goes only to the hosted cloud or to the origin `LEPTOSKIT_CLOUD_URL` names; when `leptoskit.toml` alone points somewhere else, the tool refuses and says to set `LEPTOSKIT_CLOUD_URL`.
