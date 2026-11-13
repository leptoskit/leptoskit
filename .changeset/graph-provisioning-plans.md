---
"release": patch
---

# Provision declared graph hosts from a saved plan

Graph projects now support `lk infra plan`, `apply`, and `output`. A saved plan binds the current configuration, generated provider and edge files, provider lock, backend identity, workspace, OpenTofu version and binary plan. Changed inputs require another plan. Failed replanning invalidates the old receipt, and applying consumes it before infrastructure changes begin.

Host and network replacement is refused. An optional compute budget totals all declared hosts, including dedicated service hosts. Output reads only named public host/domain values and distinguishes provider observations from runtime checks. Graph provisioning still does not install or activate application artifacts, edge runtimes or certificates.
