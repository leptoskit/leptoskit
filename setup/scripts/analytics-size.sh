#!/usr/bin/env bash
# What the analytics browser client adds to a real leptoskit web app's first
# load (docs/agents/analytics-service.md §16): builds analytics_app's hydrate
# bundle as it is, then again with the client started and every recording
# function called, runs both through wasm-bindgen, `wasm-opt -Oz` and brotli,
# and prints the difference against the 12 KiB budget.
#
#   setup/scripts/analytics-size.sh [--enforce]
#
# With --enforce it fails over the budget. It edits analytics_app and
# Cargo.lock for the second build and restores both, so run it on a clean
# checkout. Run it in `devenv shell`, which has wasm-bindgen, wasm-opt and
# brotli.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
budget=$((12 * 1024))
app=examples/analytics/analytics_app
out="$(mktemp -d)"
trap 'git checkout -- "$app" Cargo.lock; rm -rf "$out"' EXIT

target="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"

build() {
	cargo build "$@" --release --target wasm32-unknown-unknown -p analytics_app --lib --no-default-features --features hydrate
}

# Brotli bytes of the optimised wasm plus its JavaScript glue.
measure() {
	local name="$1"
	wasm-bindgen --target web --out-dir "$out/$name" "$target/wasm32-unknown-unknown/release/analytics_app.wasm"
	wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
		--enable-mutable-globals "$out/$name/analytics_app_bg.wasm" -o "$out/$name/opt.wasm"
	echo $(($(brotli -c -q 11 "$out/$name/opt.wasm" | wc -c) + $(brotli -c -q 11 "$out/$name/analytics_app.js" | wc -c)))
}

build --locked
base=$(measure base)

python3 - "$app" <<'PY'
import sys
app = sys.argv[1]
path = f"{app}/Cargo.toml"
manifest = open(path).read()
anchor = 'analytics_service = { path = "../analytics_service" }\n'
assert anchor in manifest
open(path, "w").write(manifest.replace(anchor, anchor + 'leptoskit_analytics = { workspace = true, features = ["web"] }\n', 1))
path = f"{app}/src/lib.rs"
source = open(path).read()
anchor = "\tleptos::mount::hydrate_body(app::App);"
assert anchor in source
calls = """\t{
\t\tuse leptoskit_analytics::client::web;
\t\tuse leptoskit_analytics::props;
\t\tweb::init(web::Options::default());
\t\tweb::capture_pageview(props! {});
\t\tweb::capture("signed_up", props! { "plan" => "pro", "seats" => 3 });
\t\tweb::identify("user-1", props! { "email" => "ada@example.com" });
\t\tweb::group("company", "acme", props! {});
\t\tweb::register(props! { "flavour" => "beta" });
\t\tweb::alias("ada");
\t\tweb::set(props! { "plan" => "pro" }, props! {});
\t\tweb::opt_out();
\t\tweb::opt_in();
\t\tweb::reset();
\t}
"""
open(path, "w").write(source.replace(anchor, calls + anchor, 1))
PY
build
with=$(measure with)

added=$((with - base))
echo "analytics_app: $base bytes without the client, $with with it (brotli, wasm and JS)"
echo "the client adds $added bytes; the budget is $budget"
if [ "${1:-}" = "--enforce" ] && [ "$added" -gt "$budget" ]; then
	echo "over the budget by $((added - budget)) bytes" >&2
	exit 1
fi
