#!/usr/bin/env bash
# What the analytics browser client adds to a real leptoskit web app's first
# load (docs/agents/analytics-service.md §16): builds analytics_app's hydrate
# bundle as it is, then again with the client started and every recording
# function called, runs both through wasm-bindgen, `wasm-opt -Oz` and brotli,
# and prints the difference against the 12 KiB budget.
#
#   setup/scripts/analytics-size.sh [--enforce | --self-test]
#
# With --enforce it fails over the budget; --self-test only checks that
# nested JavaScript is counted, which every run checks first. It edits analytics_app and
# Cargo.lock for the second build and restores both from git, so it refuses
# to start when either has uncommitted changes. Run it in `devenv shell`,
# which has wasm-bindgen, wasm-opt and brotli.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
budget=$((12 * 1024))
app=examples/analytics/analytics_app

# The second build edits these and the exit trap restores them from git, so
# uncommitted changes there would be lost: refuse before touching anything.
if ! git diff --quiet HEAD -- "$app" Cargo.lock; then
	echo "analytics-size: $app or Cargo.lock has uncommitted changes; commit or stash them first" >&2
	exit 1
fi

out="$(mktemp -d)"
trap 'git checkout -- "$app" Cargo.lock; rm -rf "$out"' EXIT

target="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"

build() {
	cargo build "$@" --release --target wasm32-unknown-unknown -p analytics_app --lib --no-default-features --features hydrate
}

# What a browser downloads from wasm-bindgen's output directory $1 whose
# main module is $2, brotli-compressed file by file, as served:
# "<wasm> <main js> <other js> <total>". The other JavaScript is every .js
# file below the directory, at any depth, such as the snippets
# `#[wasm_bindgen(inline_js)]` writes to snippets/<crate>/inline0.js, which
# the main module imports. Only opt.wasm counts as the wasm: it replaces the
# unoptimised one.
served() {
	local dir="$1" main="$2" wasm js=0 other=0 file size
	wasm=$(brotli -c -q 11 "$dir/opt.wasm" | wc -c)
	while IFS= read -r -d '' file; do
		size=$(brotli -c -q 11 "$file" | wc -c)
		if [ "$file" = "$dir/$main" ]; then
			js=$((js + size))
		else
			other=$((other + size))
		fi
	done < <(find "$dir" -type f -name '*.js' -print0)
	echo "$((wasm)) $((js)) $((other)) $((wasm + js + other))"
}

# A JavaScript file nested below the output, as an inline_js snippet is,
# must count: without this, the client could move code into a snippet and
# seem to shrink.
self_test() {
	local dir
	dir="$(mktemp -d)"
	head -c 4096 /dev/urandom >"$dir/opt.wasm"
	echo 'import { f } from "./snippets/probe/inline0.js"; f();' >"$dir/app.js"
	read -r _ _ before total_before < <(served "$dir" app.js)
	mkdir -p "$dir/snippets/probe"
	head -c 2048 /dev/urandom | base64 >"$dir/snippets/probe/inline0.js"
	read -r _ _ after total_after < <(served "$dir" app.js)
	rm -rf "$dir"
	if [ "$before" -ne 0 ] || [ "$after" -le 0 ] || [ "$total_after" -ne $((total_before + after)) ]; then
		echo "analytics-size: a nested .js sidecar isn't counted ($before, $after, $total_before, $total_after)" >&2
		exit 1
	fi
}
self_test
if [ "${1:-}" = "--self-test" ]; then
	echo "analytics-size: nested .js sidecars are counted"
	exit 0
fi

# Builds wasm-bindgen's output for the last analytics_app build in
# $out/$1, and prints what's served from it.
measure() {
	local name="$1"
	wasm-bindgen --target web --out-dir "$out/$name" "$target/wasm32-unknown-unknown/release/analytics_app.wasm"
	wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
		--enable-mutable-globals "$out/$name/analytics_app_bg.wasm" -o "$out/$name/opt.wasm"
	served "$out/$name" analytics_app.js
}

build --locked
read -r base_wasm base_js base_other base < <(measure base)

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
read -r with_wasm with_js with_other with < <(measure with)

added=$((with - base))
echo "analytics_app without the client: wasm $base_wasm + main js $base_js + other js $base_other = $base (brotli, per file)"
echo "analytics_app with the client:    wasm $with_wasm + main js $with_js + other js $with_other = $with"
echo "the client adds $added bytes; the budget is $budget"
if [ "${1:-}" = "--enforce" ] && [ "$added" -gt "$budget" ]; then
	echo "over the budget by $((added - budget)) bytes" >&2
	exit 1
fi
