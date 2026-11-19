# Cloud platform generator

An unpublished, offline operator tool: one typed TOML declaration produces inspectable OpenTofu JSON, Kubernetes add-ons and address-resolved node bootstrap documents. Nothing has been provisioned with it. Follow the [operator runbook](../../docs/agents/cloud-platform.md) for installation order, required Secrets, placement, upgrades and acceptance checks.

Run commands through the repository shell:

```sh
CARGO_BUILD_JOBS=4 devenv shell -- cargo run -p leptoskit_cloud_platform -- check --config crates/leptoskit_cloud_platform/examples/launch.toml
CARGO_BUILD_JOBS=4 devenv shell -- cargo run -p leptoskit_cloud_platform -- plan-summary --config crates/leptoskit_cloud_platform/examples/growth.toml
CARGO_BUILD_JOBS=4 devenv shell -- cargo run -p leptoskit_cloud_platform -- generate --config cloud-platform.toml --out generated
CARGO_BUILD_JOBS=4 devenv shell -- cargo run -p leptoskit_cloud_platform -- bootstrap --config cloud-platform.toml --outputs tofu-output.json --out generated
```

Each generated `region/cluster` directory is an independent OpenTofu root. The operator runs `tofu init/plan/apply` separately and exports `tofu output -json` for bootstrap. Existing-only clusters omit `--outputs`; multiple clusters supply a JSON object keyed by `region/cluster`, with each value containing that cluster's outputs. No command executes OpenTofu, Kubernetes tools or SSH. Generated ownership journals refuse foreign or locally edited files; interrupted writes retain both old and intended contents. A stale `.platform.lock` or `*.platform-tmp` requires operator inspection before removal. Removed topology files are retained, never deleted automatically.

Bootstrap targets Ubuntu 24.04 x86-64 with IPv4 pod/service networks. Copy the generated `.sh` document to its corresponding node, supply the shared join token in `/etc/rancher/k3s/join-token` with root-only permissions, then run the document as root. Start the first server before the remaining nodes. Cloud-init wrappers contain the same script for operators applying phase two through cloud-init; the original VM user-data only hardens the host. Tokens never enter generated files or arguments.

Servers share one provider and private network. Hetzner cloud servers and same-provider agents attach to a generated private subnet. Multiple existing servers require an operator-built private link. Other agents join over the public WireGuard overlay. [k3s does not support embedded-etcd HA over its multicloud overlay](https://docs.k3s.io/networking/distributed-multicloud). Registration uses the configured DNS name or the first server's public address; additional servers always join through its private address. One server remains a single point of failure.

Provider firewalls restrict cloud nodes; nftables also restricts host input on every node because [Hetzner firewalls do not filter private networks](https://docs.hetzner.com/cloud/firewalls/faq/). Outbound connections are allowed. Forwarding remains owned by k3s's network-policy controller. [k3s uses reverse tunnels for kubelet access](https://docs.k3s.io/installation/requirements); metrics-server is disabled, so no public kubelet port is opened. Bundled Traefik and ServiceLB are disabled; add-ons own pinned ingress placement and HTTPS host-port policy. Build taints use `NoSchedule`; other pools use `PreferNoSchedule` so bundled DNS and storage can start before add-on placement exists. Tenant isolation also requires the runtime's default-deny policies and admission settings.

Pins: k3s `v1.35.10+k3s1` (official release and SHA-256 in source), gVisor `20261116.0` (official SHA-512 URL in each example), hcloud `1.70.2` and Cloudflare `5.28.1` (the versions already validated by `lk infra`). Add-on example pins come from the official chart repositories: CloudNativePG `0.29.2`, KEDA `2.21.1`, HTTP add-on `0.16.0`, cert-manager `v1.21.3`, Traefik `42.4.1`, Zot `0.1.134`. Overridden chart keys were checked against those versions; live runtime compatibility remains unvalidated. HTTPS mode requires an operator-supplied official dual-stack Cloudflare range snapshot and an ingress-pool node. Object storage is assumed to exist.

Add-ons include CloudNativePG and its shared database with WAL/base backups, KEDA and its HTTP interceptor, cert-manager DNS-01 certificates, Traefik, the cloudflared connector and authenticated S3-backed Zot. The generated required-secrets ConfigMap names operator inputs and controller-created outputs. No secret value is generated. Nodes require an operator-supplied root-only `/etc/rancher/k3s/registries.yaml` before bootstrap. The runbook records every chart override and its verification source.

No live cluster, WireGuard, gVisor, provider firewall or boot sequence has been exercised. Generation tests and snapshots establish file contents, not working provisioned infrastructure. The price catalogue and cost model remain unimplemented.
