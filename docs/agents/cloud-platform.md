# Cloud platform operations

The platform generator prepares an operator-owned k3s installation. It writes files; it does not create accounts, run OpenTofu, install nodes or contact Kubernetes. No cluster has been provisioned with this procedure. Keep a private acceptance record with the revision, reviewed configuration, provider IDs and the result of each check below.

## Generated files and topology

Each `region/cluster` directory is an independent OpenTofu root. Phase one writes `main.tf.json` and `addons/*.yaml`. The YAML files use JSON syntax, which is a YAML 1.2 subset. Numbered names separate namespaces, secret inventory, policy, charts, dependent custom resources and the tunnel connector. Do not apply the whole directory in one operation: controllers and their CRDs must become ready first.

Phase two writes a bootstrap shell document per node and a cloud-init wrapper for each managed VM. Cloud VM user data from phase one only hardens the host. Bootstrap needs the addresses returned by OpenTofu; existing-only clusters need no outputs file. The generator's ownership journals refuse foreign and locally edited files. Removed topology files stay on disk: review stale files before applying a regenerated directory.

The launch example is one existing Ubuntu 24.04 x86-64 box, ordered separately. It runs tenants and platform services together to avoid paying for idle control-plane VMs. A failed disk or host stops the whole service. One database instance and local-path storage provide no redundancy.

The growth example has three Hetzner Cloud servers sharing a generated private subnet, three existing tenant workers and one exclusive build worker. Platform pods use the platform pool. Three control-plane servers protect etcd quorum, but the example still has one database instance and node-local storage. It does not establish database or registry availability.

Components select the platform pool when it exists, otherwise the tenant pool, otherwise the ingress pool. Traefik selects ingress first, then platform, then tenant. All selected pools get their exact `PreferNoSchedule` toleration. No platform component tolerates the build pool's `NoSchedule` taint. At least one non-build node is required.

## Networking and isolation

[k3s cannot run embedded-etcd HA over its multicloud overlay](https://docs.k3s.io/networking/distributed-multicloud). Servers share a provider and private network. For several existing servers, build that private link yourself and supply every private address. Hetzner Cloud servers and same-provider agents attach to the generated private network; other workers use the public WireGuard overlay. Keep the first server's registration address stable, or configure a DNS registration name. The generator creates no registration load balancer.

The generated provider firewall and host nftables input rules admit administrator SSH/API access, private server API/supervisor, etcd and kubelet traffic, agent-to-server TCP 6443 and peer WireGuard UDP 51820. There is no public kubelet port: k3s uses reverse tunnels, and bundled metrics-server is disabled. HTTPS opens 443 only on ingress nodes, from the configured official Cloudflare ranges. Tunnel mode opens no application port. Host input filtering supplements the provider firewall because Hetzner does not filter private-network traffic. Outbound traffic remains allowed.

Flannel's `wireguard-native` backend transports pod traffic; [k3s's embedded kube-router policy controller](https://docs.k3s.io/networking/networking-services#network-policy-controller) independently enforces NetworkPolicy through iptables. Bootstrap does not disable it or flush its tables. This is the documented enforcement mechanism, not evidence of an isolation drill on a live cluster.

Platform ingress policies exclude namespaces labeled `app.kubernetes.io/managed-by: leptoskit-cloud`. Exceptions admit those tenants to database TCP 5432 and CoreDNS TCP/UDP 53. Other trusted infrastructure namespaces can communicate, including API webhooks, Traefik to the HTTP interceptor, and the interceptor to tenant services. Tenant namespaces must be named `lk-<20 hex>` and carry that label before workloads start. Tenant credentials must not allow removing the label, editing platform policies or creating host-network pods.

The tenant runtime owns default-deny ingress/egress, denied egress CIDRs and the ingress exceptions inside each tenant namespace. Permit Traefik from kube-system and the HTTP interceptor from keda into the application port; permit outbound DNS and the shared database port. This generator cannot install policies in future namespaces. Platform policies alone do not establish tenant-to-tenant isolation. The `gvisor` RuntimeClass selects containerd handler `runsc`; tenants select it, while infrastructure controllers use the normal runtime.

## Edge and registry decisions

Bundled Traefik and ServiceLB are disabled. The pinned Traefik chart creates service `traefik` in `kube-system`, port 80. The OpenTofu tunnel targets `http://traefik.kube-system.svc.cluster.local:80`. Two cloudflared connector replicas read their tunnel token from a Kubernetes Secret. They add resilience to a pod restart, not to failure of the launch host.

Tunnel mode uses a ClusterIP service and trusts forwarded headers from the cluster pod CIDR on the HTTP entrypoint; tenant pods cannot reach that entrypoint under the destination policy. HTTPS mode uses a DaemonSet with host port 443 only on ingress nodes, a ClusterIP service for internal access, and Cloudflare forwarded-header ranges. A separate pod policy admits those ranges on target port 8443 because host-port DNAT can bypass host INPUT rules. Verify source preservation on the real nodes. Configure Cloudflare origin TLS verification and confirm the presented certificate before admitting traffic.

cert-manager solves DNS-01 through Cloudflare. Issuer `apps` creates an ACME account key, and `apps-wildcard` creates a certificate for `*.apps_domain` in kube-system. A Traefik TLSStore reads that Secret as its default certificate. With `custom_domains = "cloudflare-saas"`, Cloudflare owns customer certificates: nothing else is generated for them. With `cert-manager`, the later domain controller must create each customer Certificate; no customer domains are known here.

Zot runs only in the edge cluster, at `registry.apps_domain`, reached through the apps wildcard DNS/tunnel. It uses S3 blob storage with TLS verification and path-style requests, one persistent local cache on the configured storage class, and htpasswd authentication for reads and writes. Its own certificate is issued in the registry namespace. The distribution S3 driver's standard AWS credential chain reads Secret-backed environment variables. There is no anonymous access policy. An external registry skips these resources.

Every node must already have a root-owned mode-0600 `/etc/rancher/k3s/registries.yaml` before bootstrap. Bootstrap checks that file and preserves it. Following the [k3s registry configuration](https://docs.k3s.io/installation/private-registry), use `configs.<registry hostname>.auth.username` and `auth.password` for a registry account, plus the appropriate TLS CA if needed. For Zot the hostname is `registry.apps_domain`; otherwise use `registry.host`. The generated Zot configuration authenticates users without separating push and pull roles; every authenticated account has both permissions. Account-specific authorization remains unimplemented. Prefer a read-only node account when an external registry supports one. Keep TLS verification enabled. Supply the file through the approved secret store, never a generated manifest or Git. k3s loads it into containerd on startup; restart each node's k3s service after rotating it. This also lets the initial infrastructure pull public controller images before Zot is ready.

## Required Secrets

The generated `required-secrets` ConfigMap lists names, namespaces, keys, purpose, source and owner. It contains no values. Entries owned by cert-manager are outputs, not Secrets to precreate.

| Namespace                       | Secret                         | Keys                          | Source and purpose                                                          |
| ------------------------------- | ------------------------------ | ----------------------------- | --------------------------------------------------------------------------- |
| database                        | configured object-storage name | configured access/secret keys | S3 credentials for WAL and base backups                                     |
| registry, edge cluster with Zot | same object-storage name       | same keys                     | Credentials for the registry bucket                                         |
| registry, with Zot              | `registry.auth_secret`         | `htpasswd`                    | Operator-created bcrypt htpasswd file; registry authentication              |
| cert-manager, edge cluster      | `addons.cloudflare_api_secret` | `api-token`                   | Cloudflare API token restricted to DNS edit and zone read for the apps zone |
| kube-system, tunnel edge        | `cloudflare-tunnel`            | `token`                       | Sensitive `tunnel_token` OpenTofu output                                    |
| cert-manager, edge cluster      | `apps-acme-account`            | `tls.key`                     | Created by cert-manager                                                     |
| kube-system, edge cluster       | `apps-wildcard-tls`            | `tls.crt`, `tls.key`          | Created by the wildcard Certificate                                         |
| registry, with Zot              | `registry-tls`                 | `tls.crt`, `tls.key`          | Created by the registry Certificate                                         |

With environment-based S3 references, create `object-storage` with keys `ACCESS_KEY_ID` and `SECRET_ACCESS_KEY` in each consumer namespace. The named environment variables are the operator's source; generation does not read them. With Kubernetes references, copy the configured Secret into each consumer namespace. Give it bucket-scoped permissions for the intended workload. An external registry's pull Secret belongs to future workload namespaces and is not an add-on dependency; the runtime must create it there.

Resolve secrets once through Monosecret in the private operator session. Create Secrets from private files, for example `kubectl -n cert-manager create secret generic cloudflare-dns --from-file=api-token=DNS_TOKEN_FILE`. Substitute the configured name. Do not put values in command arguments, shell history, output captures or the acceptance record.

## Install from an empty account

1. Create the Hetzner and Cloudflare accounts, apps DNS zone, SSH keys, and S3-compatible buckets named by the configuration. Order existing hosts separately. Confirm Ubuntu 24.04 x86-64, public addresses, administrator access and any required private links. Set a spending limit outside this generator. Prepare the configured StorageClass; local-path is supplied by k3s.
2. Copy an example to `cloud-platform.toml`, replace documentation identifiers and addresses, and review its node roles, pools, CIDRs, chart pins and failure domains. Keep provider tokens outside the configuration.
3. Run the local generator commands:

   ```sh
   RUSTUP_AUTO_INSTALL=0 CARGO_BUILD_JOBS=4 devenv shell -- cargo run -p leptoskit_cloud_platform -- check --config cloud-platform.toml
   RUSTUP_AUTO_INSTALL=0 CARGO_BUILD_JOBS=4 devenv shell -- cargo run -p leptoskit_cloud_platform -- plan-summary --config cloud-platform.toml
   RUSTUP_AUTO_INSTALL=0 CARGO_BUILD_JOBS=4 devenv shell -- cargo run -p leptoskit_cloud_platform -- generate --config cloud-platform.toml --out generated
   ```

4. In each generated cluster directory run `tofu init`, `tofu plan -out=platform.plan`, review that saved plan, then `tofu apply platform.plan`. Configure protected state storage first. Export `tofu output -json > tofu-output.json` into private mode-0600 storage; it contains a sensitive tunnel token. Never commit state, plans or outputs.
5. Run `RUSTUP_AUTO_INSTALL=0 CARGO_BUILD_JOBS=4 devenv shell -- cargo run -p leptoskit_cloud_platform -- bootstrap --config cloud-platform.toml --outputs tofu-output.json --out generated`. Omit outputs for existing-only clusters. For several clusters supply one JSON object keyed by `region/cluster`, each containing that cluster's outputs.
6. Verify host fingerprints through the provider. Install the same shared k3s join token on every node at `/etc/rancher/k3s/join-token`, root-owned mode 0600. Prepare the registry file described above. Transfer each bootstrap document only to its matching node.
7. Run the first server's bootstrap as root. It installs verified k3s/gVisor artifacts and starts `leptoskit-k3s.service`. Check its journal and `systemctl is-active leptoskit-k3s`. Join the remaining servers, then agents by running their documents. Additional servers join privately. Copy the kubeconfig through the approved administrator channel, set its API endpoint to the configured registration address, and protect it.
8. Check `kubectl get nodes -o wide` and `kubectl get nodes -L leptoskit.dev/pool`; all declared nodes must be Ready with the expected pool, addresses and taints.
9. In the cluster's `addons` directory apply the `00-*.yaml` namespaces, create operator-owned Secrets, then apply `01-*.yaml` and `10-*.yaml`. Inspect `kubectl get runtimeclass gvisor -o yaml` and `kubectl get networkpolicy -A`.
10. Apply `20-*.yaml`. Wait for chart installation and controller readiness using the checks below. Only then apply `21-*.yaml` for the HTTP add-on; it depends on KEDA CRDs.
11. After the HTTP controller is ready, apply `30-*.yaml`. Wait for database readiness and certificate issuance. Finally apply `40-*.yaml` for the tunnel, when generated. Check the public route and registry authentication before installing the control plane.

Use individual `kubectl apply -f FILE` operations for each numbered file group above. Do not rely on filesystem iteration order or a single apply retry to coordinate CRD registration.

## Health and readiness evidence

```sh
kubectl -n kube-system get helmcharts
kubectl -n kube-system get jobs,pods
kubectl -n cnpg-system rollout status deployment/cloudnative-pg --timeout=300s
kubectl -n cert-manager rollout status deployment/cert-manager --timeout=300s
kubectl -n cert-manager rollout status deployment/cert-manager-webhook --timeout=300s
kubectl -n cert-manager rollout status deployment/cert-manager-cainjector --timeout=300s
kubectl -n keda rollout status deployment/keda-operator --timeout=300s
kubectl -n keda rollout status deployment/keda-operator-metrics-apiserver --timeout=300s
kubectl -n keda rollout status deployment/keda-admission-webhooks --timeout=300s
kubectl -n keda get deployments,services
kubectl -n keda wait --for=condition=Available deployment --all --timeout=300s
kubectl -n kube-system get deployment,daemonset,service -l app.kubernetes.io/name=traefik
kubectl -n database wait --for=condition=Ready cluster/shared --timeout=600s
kubectl -n database get cluster shared -o yaml
kubectl -n database get scheduledbackup,backup
kubectl wait --for=condition=Ready clusterissuer/apps --timeout=300s
kubectl -n kube-system wait --for=condition=Ready certificate/apps-wildcard --timeout=600s
kubectl -n registry wait --for=condition=Ready certificate/registry --timeout=600s
kubectl -n registry rollout status statefulset/zot --timeout=300s
kubectl -n kube-system rollout status deployment/cloudflared --timeout=300s
kubectl -n kube-system logs deployment/cloudflared --tail=50
```

Skip registry and connector checks when those resources are not generated. For Traefik use `rollout status deployment/traefik` in tunnel mode or `rollout status daemonset/traefik` in HTTPS mode. Inspect failed Helm installer Jobs before retrying; chart existence alone does not mean installation succeeded.

Confirm the HTTP interceptor proxy Service has ready endpoints and is reachable from Traefik on port 8080. Chart 0.16.0 uses InterceptorRoute; the tenant runtime must generate compatible routing resources. Exercise a disposable app's sleep and wake cycle before declaring sleeping apps supported.

For Postgres, require healthy instances, successful WAL archiving and a completed base backup, then restore into a disposable cluster and verify data. The daily schedule is 02:00 UTC with a six-field CNPG cron expression. Backup prefixes include region and cluster to avoid collisions. Later, the control plane will create one role/database per environment using privileged SQL through a separate administrative credential, or CNPG managed roles and Database resources; this generator creates neither tenant databases nor tenant grants.

Run a disposable labeled tenant pod with gVisor. Verify DNS and database connectivity, rejection of platform API/registry/interceptor connections, rejection of forbidden egress and tenant-to-tenant access, and ingress through both Traefik and the interceptor. An object listing cannot prove packet filtering.

Check unauthenticated registry push and pull both fail; then push and pull with valid credentials, including a node-side containerd pull. Check large layers through Cloudflare against its upload limits. No such checks have been run here.

## Growth, replacement and upgrades

To add a node, add a stable node group or increase its count, regenerate, review/apply the infrastructure plan, export fresh outputs and regenerate bootstrap for every node. Firewall peers change on existing hosts too: review and rerun their bootstrap documents in a maintenance window. Bootstrap restarts k3s, so update one server at a time while preserving etcd quorum. Supply token and registry files to the new node, run its bootstrap, and confirm Ready, pool placement and cross-node policy enforcement before scheduling workloads.

To replace a worker, first add replacement capacity, drain the old node with its disruption budgets, migrate or restore node-local volumes, and verify workloads on the replacement. Remove the old Kubernetes node and infrastructure only after that evidence. A local-path database/cache cannot simply follow a drained pod to another host. Replacing a server also requires an etcd snapshot, quorum and the k3s server-removal procedure; never remove two members together. Replacement protection intentionally makes provider-owned servers a deliberate operation.

Change one chart pin at a time. Read its upgrade notes, fetch that exact chart, verify every overridden key in its values and rendered templates, regenerate and review snapshots, then test in a disposable cluster. Wait for its CRDs/webhooks and workloads before dependent upgrades. k3s and gVisor pins are coupled to verified checksums in Rust; update the artifact and checksum together and roll nodes individually. Keep the previous manifests and recoverable backups; CRD and database upgrades may prevent rollback.

Back up encrypted OpenTofu state and lockfiles, the non-secret operator configuration and generator revision, k3s etcd snapshots and server token, node registry credentials, operator Secrets, ACME account key, CNPG base backups and WAL, registry bucket and persistent cache, and any tenant persistent volumes. Keep recovery credentials separately from the cluster. Test recovery from loss of the launch host and of one growth server.

## Verified chart inputs

The following is the complete override inventory. Placement means `nodeSelector.leptoskit.dev/pool` and `tolerations` with key `leptoskit.dev/pool`, operator `Equal`, selected pool value and effect `PreferNoSchedule`.

| Chart pin            | Values set                                                                                                                                                                                                                                                                                                  | Verification source                                                                                                                                                                                                                                |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CloudNativePG 0.29.2 | placement                                                                                                                                                                                                                                                                                                   | [Tagged values](https://raw.githubusercontent.com/cloudnative-pg/charts/cloudnative-pg-v0.29.2/charts/cloudnative-pg/values.yaml)                                                                                                                  |
| KEDA 2.21.1          | root placement and placement under `operator`, `metricsServer`, `webhooks`                                                                                                                                                                                                                                  | [Tagged values](https://raw.githubusercontent.com/kedacore/charts/v2.21.1/keda/values.yaml)                                                                                                                                                        |
| KEDA HTTP 0.16.0     | placement under `operator`, `scaler`, `interceptor`; `interceptor.directPodRouting=false` to retain Service routing                                                                                                                                                                                         | [Published chart archive](https://kedacore.github.io/charts/keda-add-ons-http-0.16.0.tgz), values.yaml                                                                                                                                             |
| cert-manager v1.21.3 | root placement and placement under `webhook`, `cainjector`, `startupapicheck`; `crds.enabled=true`                                                                                                                                                                                                          | [Tagged values](https://raw.githubusercontent.com/cert-manager/cert-manager/v1.21.3/deploy/charts/cert-manager/values.yaml)                                                                                                                        |
| Traefik 42.4.1       | placement; `fullnameOverride=traefik`; `service.type=ClusterIP`; `ports.web.forwardedHeaders.trustedIPs` (pod CIDR in tunnel mode, Cloudflare ranges otherwise); `ports.websecure.forwardedHeaders.trustedIPs` (Cloudflare ranges); HTTPS only: `deployment.kind=DaemonSet`, `ports.websecure.hostPort=443` | [Tagged values](https://raw.githubusercontent.com/traefik/traefik-helm-chart/v42.4.1/traefik/values.yaml)                                                                                                                                          |
| Zot 0.1.134          | placement; `fullnameOverride=zot`; `service.type=ClusterIP`, `service.port=5000`; `mountConfig=true`; `configFiles.config.json`; `externalSecrets[].secretName/mountPath`; `persistence=true`; `pvc.storageClassName`; `env[].name/valueFrom.secretKeyRef.name/key`                                         | [Tagged values](https://raw.githubusercontent.com/project-zot/helm-charts/zot-0.1.134/charts/zot/values.yaml), [StatefulSet template](https://raw.githubusercontent.com/project-zot/helm-charts/zot-0.1.134/charts/zot/templates/statefulset.yaml) |

Zot's config.json sets `distSpecVersion`, `storage.rootDirectory`, `storage.dedupe`, S3 driver name/region/regionendpoint/bucket/secure/skipverify/forcepathstyle, HTTP address/port/auth.htpasswd.path and log level. See [Zot storage](https://zotregistry.dev/v2.1.11/articles/storage/), [configuration](https://zotregistry.dev/v2.1.11/admin-guide/admin-configuration/), and the [distribution driver](https://raw.githubusercontent.com/distribution/distribution/v3.0.0/registry/storage/driver/s3-aws/s3.go). CNPG Cluster backup, affinity and environment fields were checked against the [v1.30.2 API](https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/v1.30.2/api/v1/cluster_types.go). cloudflared is pinned separately to [2026.11.0](https://github.com/cloudflare/cloudflared/releases/tag/2026.11.0).

## Not validated and not done

No provider apply, node boot, live chart install, server-side schema check, WireGuard connectivity, gVisor workload, NetworkPolicy enforcement, public TLS, registry authentication/S3 round trip, sleep/wake cycle or backup restore has been exercised. Offline snapshots prove generated content and determinism. The optional client-side kubectl dry run was not run; no cluster credentials were used.

The control-plane deployment, tenant databases/roles, customer-domain Certificates, tenant admission/RBAC, price catalogue and cost model are outside this half of the Platform slice. Resource bodies are constructed as JSON values rather than generated Kubernetes API types; offline tests do not validate every CRD field against a live API. Multi-cluster tenant traffic routing, production HA storage, registry account provisioning and credential rotation automation remain operator/runtime work. Do not describe this as a production-ready cloud until the acceptance checks above have recorded results.
