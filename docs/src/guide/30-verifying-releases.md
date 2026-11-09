# Verifying a release

Anyone using your app has to trust that the code running it is the code you published. When your repository is public, leptoskit lets them check that for themselves. You build your backend from a commit on GitHub with `leptoskit verify build`; anyone else runs `leptoskit verify reproduce` to fetch that commit, rebuild it, and compare the result with your executable byte for byte. A reviewer, or a coding agent they point at your repository, then knows the code they read is the code you shipped.

This page covers backend executables built for Linux, and the websites and APIs that serve them. The commands run on macOS and Linux; building and reproducing need Docker or Podman. They prove what a release contains and which origins you approve to serve it; they do not prove which executable a live server is running, and they say nothing about whether the code is safe.

## Building a backend people can verify

Your repository must be public on GitHub, with `Cargo.lock` committed. Write a recipe naming the commit and how to build it:

```json
{
	"repository": "https://github.com/example/backend",
	"commit": "1234567890123456789012345678901234567890",
	"builder_image": "docker.io/library/rust@sha256:ed2c95ca377d6571b124b05e75e01207973ded639dce7d5542eb7bcd797e32aa",
	"target": "aarch64-unknown-linux-gnu",
	"package": "backend",
	"binary": "server",
	"features": [],
	"no_default_features": false,
	"rustflags": [],
	"infra": "deploy"
}
```

Use your own repository, the full 40-character commit ID, and your package and binary names. Every field except `infra` is required, including empty lists and `false`, so nothing is left to a default:

- `builder_image` is the container that compiles your code, pinned by its digest. The example is the official Rust 1.99.1 image. A tag such as `rust:latest` is refused, because it can point to a different compiler tomorrow. If your repository has a `rust-toolchain.toml`, the image must already contain that toolchain: nothing is downloaded while compiling.
- `target` is `x86_64-unknown-linux-gnu` or `aarch64-unknown-linux-gnu` (or their `-musl` variants), and must match your machine's processor, since the compiler runs natively in the container. Use an x86-64 machine for an x86-64 server.
- `features` and `no_default_features` are passed to Cargo as written. `rustflags` is the complete list of compiler flags: rustflags in your repository's Cargo configuration are not used.
- `infra` optionally names a directory in the commit, such as your deployment configuration, to publish alongside the source.

Then build:

```sh
leptoskit verify build \
  --recipe recipe.json \
  --secret DATABASE_URL --secret API_KEY \
  --out release
```

The command:

1. fetches the commit from your public repository without credentials (pass `--checkout <clone>` to read it from a local clone instead), and writes its exact files, executable bits included, to `release/bundle/source/`. Submodules and symbolic links are refused, so every source is in the commit;
2. downloads every dependency `Cargo.lock` names into `release/bundle/dependencies/`. This is the only step with network access, and your source is read-only during it;
3. compiles the binary twice in fresh containers with networking disabled, the source and dependencies read-only, and a fixed environment, then requires the two executables to be identical. A difference fails the build, because nobody else could reproduce it;
4. writes `release/inventory.json`, which pins every file in the bundle, and `release/provenance.json`, a standard [SLSA provenance](https://slsa.dev/spec/v1.0/provenance) statement naming the commit, builder image, lockfile, compiler version, and executable digest.

`--secret` records the names of the environment variables your server reads secrets from, in `bundle/secrets.json`; their values are never read. The command prints the executable's and the inventory's SHA-256. Publish the bundle and inventory with your release, and publish the inventory SHA-256 somewhere people can authenticate it, such as your release notes or a signed tag. The files themselves are not signed.

## Reproducing a release

Anyone with the bundle, its inventory, and the inventory SHA-256 from a source they trust can check the release:

```sh
leptoskit verify reproduce \
  --bundle release/bundle \
  --manifest release/inventory.json \
  --sha256 <trusted-inventory-sha256>
```

Each line of the output is one claim that held:

```text
bundle matches the pinned inventory (4238 entries)
source/ and infra/ are exactly https://github.com/example/backend at commit 1234567…
dependencies/ is exactly what source/Cargo.lock resolves to
artifacts/backend was rebuilt byte for byte from source/ and dependencies/ in docker.io/library/rust@sha256:…
Not verified: which executable a live server runs, or whether the code is safe.
```

The command fetches the commit from the repository named in the recipe, downloads the dependencies the lockfile names, and compiles the binary once with networking disabled. Any difference fails it and names the first file that differs: changed source, a modified dependency, or an executable that the published source does not produce.

`--checkout <clone>` compares the source with a local clone instead of fetching it, and says so in its output. `--offline` skips both network steps and only rebuilds the executable from the bundle's own files; the skipped claims are reported as not checked. Rebuilding needs a machine with the same processor architecture as the recipe's target.

## Proving which websites are yours

Your source is public, so anyone can deploy a copy of it, or copy your website, and claim to be you. A deployment policy says which origins serve your deployment and which releases they may run, signed with a key only you hold. A copy at another address fails the check, a forged policy fails the signature, and an old policy cannot replace a newer one.

Create a key once, and publish its public half in your repository, for example in your readme or `SECURITY.md`, so people get it from your code rather than from a website:

```sh
leptoskit verify keygen --out deployment.key
```

Keep `deployment.key` out of your repository. Write a policy listing every origin and the releases `verify build` produced:

```json
{
	"format_version": 1,
	"kind": "leptoskit-deployment-policy",
	"repository": "https://github.com/example/app",
	"deployment": "production",
	"origins": ["https://example.com", "https://api.example.com"],
	"sequence": 1,
	"issued_at": 1793750400,
	"expires_at": 1796342400,
	"releases": [
		{
			"commit": "1234567890123456789012345678901234567890",
			"inventory_sha256": "<the inventory SHA-256 verify build printed>",
			"executable_sha256": "<the executable SHA-256 verify build printed>"
		}
	]
}
```

Times are Unix seconds. Increase `sequence` every time you sign a new policy, and keep the expiry short, a month or so, so a policy you have replaced stops working on its own. Origins are written exactly as `https://host`, with a port only when it isn't 443. Sign it and serve the result at `/.well-known/leptoskit/deployment.json` on every origin it lists:

```sh
leptoskit verify sign --policy policy.json --key deployment.key --out deployment.json
```

Anyone can then check an origin against the key from your repository:

```sh
leptoskit verify deployment \
  --origin https://example.com \
  --key <public key from the repository> \
  --repository https://github.com/example/app \
  --state example-production.json
```

The check fetches the policy over HTTPS and refuses it unless your key signed it, it names that repository and origin, and it has not expired. `--state` remembers the newest policy seen, so a later check refuses an older or conflicting one. The output lists the approved releases with their inventory SHA-256, which is the authenticated digest `verify reproduce` needs.

A policy is your signed statement about what your origins serve. It does not prove what the server at an origin is actually running: that needs hardware attestation of the server, which leptoskit does not provide yet.

## Checking a backend bundle

`leptoskit verify check` is the first step of `reproduce` on its own: it needs no container or network. Before comparing or reviewing a backend release, check that its source files, dependency sources, build declaration, infrastructure definitions and executable match the inventory you trust:

```sh
leptoskit verify check \
  --bundle ./backend-bundle \
  --manifest ./backend-inventory.json \
  --sha256 <trusted-inventory-sha256>
```

This is an offline file check for a prepared backend bundle, not a connection to a running backend. It applies to backend artifacts used by any client platform. A website profile also checks its inventoried browser files, but this command does not reproduce those files or verify a native client. Unix file permission bits are compared when available; checking an inventory carrying Unix permissions requires a host and extraction method that preserve those permissions.

Obtain the expected SHA-256 from independently authenticated release evidence or a previously trusted record. A checksum supplied alongside an untrusted download cannot establish who produced it. A successful check means the local files match that inventory; it does not prove that the executable was built from the included sources, that dependencies are complete, that a server runs it, or that its code is safe.

### Preparing an inventory by hand

`verify build` writes the bundle and its inventory for you. To inventory a backend built some other way, prepare a directory containing only public materials:

```text
backend-bundle/
  source/
    Cargo.toml
    Cargo.lock
    src/main.rs
  dependencies/
  infra/
  artifacts/
    backend
  build.json
  secrets.json
```

Include the complete application source in `source/`, dependency sources in `dependencies/`, infrastructure definitions in `infra/`, and the built backend executable at `artifacts/backend`. The dependency and infrastructure directories must exist, even when empty. Include other build inputs and runtime libraries under their relevant directories; the checker inventories the supplied files but does not discover missing inputs or run Cargo.

`build.json` declares the intended recipe. All fields below are required, including empty lists and `false`:

```json
{
	"repository": "https://github.com/example/backend",
	"commit": "1234567890123456789012345678901234567890",
	"builder_image": "registry.example/builder@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	"target": "x86_64-unknown-linux-gnu",
	"package": "backend",
	"binary": "backend",
	"features": [],
	"no_default_features": false,
	"rustflags": []
}
```

Replace the illustrative repository, full commit ID and builder digest with your release's values. `record` and `check` validate the declaration's format but do not contact the repository or builder, execute this recipe, or confirm that it produced the executable; `reproduce` does. Unknown recipe fields are rejected.

`secrets.json` contains names only:

```json
["DATABASE_URL", "API_KEY"]
```

Use `[]` when the backend needs no secrets. Do not include assignments or secret values. The command does not read your environment or collect working-tree files, but it cannot identify secrets hardcoded in files you supply.

Create the inventory outside the bundle:

```sh
leptoskit verify record \
  --bundle ./backend-bundle \
  --out ./backend-inventory.json
```

The command prints the inventory's SHA-256 and refuses to overwrite an existing inventory. It records file paths, lengths, hashes, Unix file permissions where supported, and directory entries. Missing, changed and extra entries—including empty directories—fail a later check. Symbolic links and special files are rejected. The backend-only inventory does not check directory permissions, ownership, timestamps, ACLs or extended attributes. The website profile additionally requires fixed file and directory modes. Use an extracted directory that is not being modified while checking it.

The inventory is unsigned. Authenticate it separately before others rely on it. Both commands state explicitly that build provenance and the live deployment were not verified.

## Require independent approval for a backend release

For a server-only app or a supported full-stack website, an operator can require reviewers to approve the complete backend image descriptor before deployment. Each reviewer checks the reproduced source, executable, image configuration, schema and RPC evidence, then signs the exact descriptor file:

```sh
lk verify keygen --out reviewer.key
lk verify approve-backend --descriptor descriptor.json --key reviewer.key --out approval.json
```

Get reviewers' public keys independently of the publisher. In the operator's managed `infra/<deployment-name>/` directory, install `backend-trust.json`:

```json
{
	"format_version": 1,
	"repository": "https://github.com/example/backend",
	"deployment": "production",
	"publisher": "<publisher public key in hex>",
	"approvers": ["<reviewer public key in hex>"],
	"threshold": 1,
	"deployment_keys": ["<deployment public key in hex>"]
}
```

The publisher cannot count as an independent reviewer. Prepare `backend-authorization.json` with `format_version: 1`, `descriptor` containing the exact descriptor text as a JSON string, `approvals` containing the approval objects, and `deployment_policy` set to an empty string. Keep the original descriptor bytes, including whitespace.

The deployment signer uses the existing `lk verify sign` command with a version 2 policy. Each release includes the existing commit, inventory and executable hashes plus a `backend` object containing `descriptor_sha256` and `engine_image_id` from that descriptor. Install the resulting signed envelope as `backend-deployment.json`. The operator then deploys with the existing descriptor and independent pin flags.

Once trust is configured, missing approvals cannot fall back to an unsigned deployment. Reuse, rollback and recovery recheck the signatures and the current policy. To restore an older image, issue a newer policy that explicitly authorizes its descriptor and image; preserve `backend-policy-state.json` so older or conflicting policies remain rejected. Protect these operator files from the release publisher.

This path targets managed Linux backend containers. Website releases require the explicit production profile below. Application service releases remain unsupported. Approval does not prove code safety, RPC compatibility, or which executable is running on a server; runtime attestation is absent.

### Keep installed clients compatible during a release

For managed Linux backends and supported websites, retain the published RPC spec used by each client version you still support. During verified packaging, compare those contracts with the spec exported by the reproduced executable:

```sh
lk infra build-verified \
  --bundle release/bundle --manifest release/inventory.json --sha256 "$INVENTORY_PIN" \
  --runtime-image "$RUNTIME_IMAGE_AT_DIGEST" --runtime-config-sha256 "$RUNTIME_CONFIG_PIN" \
  --tag registry.example/backend:next --service backend-production \
  --rpc-baseline published-rpc.json --rpc-baseline-sha256 "$RPC_BASELINE_PIN" \
  --out descriptor.json
```

Set `--service` to the destination’s exact `infra.name`. Kamal requires that image label; verified packaging includes it in the pinned image configuration. Package separately for different managed names, even when the executable is identical.

Obtain each baseline pin from your own release records or another authenticated source. Repeat both baseline flags in matching order for up to eight supported contracts. Each spec may be at most 16 MiB. Baseline files are read and pinned before the build begins. A comparison failure stops descriptor creation. Without a baseline, compatibility remains `not_checked`; `--no-rpc` cannot be combined with baselines.

The version 3 descriptor retains the exact candidate and baseline spec bytes and identifies the structural comparison algorithm. Approvals and the deployment policy bind that complete descriptor. Readers recompute the comparison from its contents when approving or consuming it, including reuse, rollback, and recovery.

To enforce the client window independently of the publisher, use version 2 of the operator's `backend-trust.json` and list the required baseline hashes:

```json
{
	"format_version": 2,
	"repository": "https://github.com/example/backend",
	"deployment": "production",
	"publisher": "<publisher public key in hex>",
	"approvers": ["<reviewer public key in hex>"],
	"threshold": 1,
	"deployment_keys": ["<deployment public key in hex>"],
	"rpc_contracts": ["<authenticated published RPC spec SHA-256>"]
}
```

Every listed contract must appear in the checked descriptor. A publisher's empty or different baseline cannot replace this operator requirement. Legacy descriptors, `not_checked`, and `not_applicable` fail this policy. Rollback also uses the current client window and its recorded descriptor. If an older release lacks a required baseline, reproduce that source with the current baselines and issue new evidence, approvals, and deployment policy for a fresh release tag. Recorded tags and their evidence cannot be overwritten. Version 1 trust files keep their previous behavior and cannot carry `rpc_contracts` requirements.

This uses the conservative declaration comparison described in [Checking compatibility](13-sharing-the-rpc.md#checking-compatibility). It covers HTTP and live wire shapes for clients on any platform. It does not prove unchanged handler behavior, authorization, runtime protocol implementations, database rollout compatibility, or native host support. Schema changes may need a separately named endpoint while older clients remain supported.

## Reproduce a website and its backend together

To check that the JavaScript, Wasm, CSS and public files in your image match your source release, add a website profile to the build recipe before running `lk verify build`:

```json
{
	"profile": "leptoskit-web-v1",
	"config_sha256": "<SHA-256 of the committed leptoskit.toml>",
	"toolchain_sha256": "<SHA-256 of the ordered builder version report>",
	"output_name": "my_app"
}
```

This object is the recipe's `website` field. Keep the other recipe fields, use empty `features` and `rustflags` and `no_default_features: false`, and configure those settings in `leptoskit.toml`. The backend package and binary must equal `app.name`; `output_name` is the client library stem used in `pkg/my_app.js`. Select a digest-pinned builder containing Rust, the `wasm32-unknown-unknown` target, `leptoskit` and `wasm-opt`. The version report is the exact stdout of `rustc -vV`, `cargo -V`, `leptoskit --version`, and `wasm-opt --version`, concatenated in that order in the builder using your source's toolchain.

This profile targets full-stack SSR or CSR websites with a Linux GNU backend on x86_64 or aarch64. It uses the ordinary production web build, including configured Wasm optimization, compression and static assets. The pinned CLI also handles the default split setting and any generated chunks. Explicit plugins, localization, islands and static generation are not supported in this profile. Application services are also unsupported. Native platform outputs are not built or verified by this command.

Build and reproduce with the same commands as the backend example. Both clean builds must produce identical backend bytes and identical website paths, modes and file contents. Packaging copies these checked files to `/app/backend` and `/app/dist`, then checks the files extracted from the image. Website files must have mode 0644 and directories mode 0755; links are rejected. Keep generated `dist`, `target`, `.leptoskit`, `web/pkg` and `public/pkg` directories out of the source commit.

The website descriptor includes the complete asset tree, configuration and builder inputs. Independent approvals and the deployment policy bind that entire descriptor and image. Missing website evidence blocks a verified release, including rollback or recovery. A hand-written inventory still proves only file integrity; supplied assets are never called reproduced until the source build has actually been repeated and compared. These checks do not certify API compatibility or attest what a running server executes.

## Check a graph build before using its files

When one project builds several frontends or services, check that the files you received are exactly the files recorded by its build. From the graph project directory, use the digest supplied through your trusted review or build channel:

```sh
lk verify graph \
  --bundle .leptoskit/graph/builds/build-EXAMPLE \
  --sha256 "$RECEIPT_SHA256"
```

This command runs on macOS and Linux and checks the receipt against the current project configuration. It checks complete file membership, contents and permissions, recipes, host placement, readiness declarations and any declared toolchain image pin. It does not execute the supplied application. The checksum delivered inside an unknown receipt is not an independent digest: replacing both files and their checksum must not change what you trust.

To prepare just the docs files for a later packaging step, create an output parent and select that unit:

```sh
mkdir -p checked-builds
lk verify graph \
  --bundle .leptoskit/graph/builds/build-EXAMPLE \
  --sha256 "$RECEIPT_SHA256" \
  --unit frontends.docs \
  --out checked-builds
```

The result names a new owner-only staging directory. Other units are not copied or restarted, and the original receipt's exact manifest remains attached. Inputs are checked again during copying; a failed check leaves no successful staging child. Readiness dependencies remain declarations that a deployment must check against the running services.

These graph receipts come from ordinary local builds. Checking or staging them does not reproduce their source, approve a release, establish the executable's target platform or deploy it. Graph activation and its release-approval integration remain unavailable; the backend and website reproduction workflows above retain their separate requirements. Windows cannot perform these exact Unix file-mode checks.

To catch a macOS executable or the wrong CPU architecture before preparing a Linux release, add an explicit format check:

```sh
lk verify graph \
  --bundle .leptoskit/graph/builds/build-EXAMPLE \
  --sha256 "$RECEIPT_SHA256" \
  --linux-architecture x86_64
```

Use `aarch64` for an ARM64 Linux host. This inspection runs on macOS or Linux without executing the supplied application or its loader. It checks selected executables for supported ELF headers, architecture, file-backed entry points, and bounded program headers. It reports the requested interpreter and shared libraries. It rejects executable stacks, writable executable segments, and embedded library search paths. Static frontend assets need no executable format check.

Produce compatible graph artifacts by running `lk build` from the same project on the intended Linux architecture, then transfer the complete receipt and obtain its digest through your trusted build channel. Packaging must consume those bytes rather than rebuild the executable. This is an ordinary build, not source reproduction. Format inspection does not prove that the target host has compatible libraries, that the program will start, or that its readiness endpoint works; those remain separate deployment checks.

## Release a static frontend to Linux

Use a static release to update documentation without installing or restarting your compiler service. Targets need Linux, systemd, Python 3, and an existing dedicated `leptoskit-edge` user and group. The operator command runs on Unix. Select static units explicitly:

```sh
lk infra release plan --unit frontends.docs --version docs-20261108 \
  --bundle /absolute/docs-bundle --sha256 "$GRAPH_RECEIPT_SHA256" \
  --trust /absolute/host-trust.json --trust-sha256 "$HOST_TRUST_SHA256" \
  --caddy /absolute/caddy-linux --caddy-sha256 "$CADDY_SHA256" \
  --plan /absolute/docs-release.json
```

Obtain digests independently of the bundle. The command checks typed infrastructure outputs and generated edge files; it does not provision hosts. Replace `plan` with `apply` and repeat the arguments to install and activate. Use `status` to inspect current pointers and journals; status does not claim public route health.

The independently maintained host trust file has this shape:

```json
{
	"hosts": {
		"web": {
			"ipv4": "203.0.113.10",
			"port": 22,
			"host_key": "ssh-ed25519 BASE64_PUBLIC_HOST_KEY",
			"identity": "/absolute/private-ssh-key",
			"architecture": "x86_64",
			"prerequisites": {
				"/etc/leptoskit/my-app/tls/docs.example.com.crt": "INDEPENDENT_CERTIFICATE_SHA256"
			},
			"tunnel": null
		}
	}
}
```

Replace example values with independently checked pins and the applied address. SSH uses the explicit root identity and strict host-key checking. Keep that identity private. Caddy runs under the dedicated account. Install certificates and matching private keys at generated paths, readable by that account. Pin each public certificate in `prerequisites`; private key contents never belong in receipts.

Tunnel ingress needs an already active cloudflared service. Set `tunnel` to an object with `unit`, `fragment` (absolute systemd unit file), and `sha256`; pin its configured input under `/etc/leptoskit/` in `prerequisites`. The release checks files and service activity but does not create the tunnel or establish external reachability. HTTPS origins retain TLS, with no HTTP fallback.

Installation uses `/var/lib/leptoskit/<infra.name>/units/<unit>/releases/<version>` and atomic `current` pointers. The supplied Caddy and exact generated configuration are installed and checked. Selected routes must serve candidate index bytes before promotion and active bytes afterward. Existing shared edge configuration must match; changes need a separate migration. Unselected service files, processes and history remain untouched. Restarting the edge can briefly interrupt requests.

For rollback, use `rollback` with the earlier release's original arguments, bundle and saved plan. Current configuration, provider bindings, host trust and runtime inputs must still pass. Retrying the same checked apply after an interrupted promotion restores previous pointers before trying again. Hosts commit separately: inspect every selected host after a transport failure.

This initial transport accepts 64 MiB per host including Caddy. Selected SSR/service units and readiness dependencies without a runtime observer fail before installation. Declaring unselected services does not block a static release. `--policy strict` fails because operator-directed deployment is not independent graph approval, source reproduction or runtime attestation.

For an SSR frontend, use an application that consumes the toolkit's `LEPTOS_SITE_ADDR` binding. A service uses `LEPTOSKIT_SERVICE_ADDR`; a plain Axum server must explicitly read that address when binding its listener. Setting `PORT` alone does not establish either contract. Executable graph releases are still unavailable: a declared address and a successful health response alone do not establish safe activation or rollback.
