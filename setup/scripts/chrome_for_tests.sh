#!/usr/bin/env bash
# Point wasm-bindgen-test at a Chrome whose version matches chromedriver.
#
# The devenv chromedriver and the Chrome on a GitHub runner are different
# majors. ChromeDriver refuses the session and names the runner's binary
# (`/opt/google/chrome/chrome`). wasm-bindgen 0.2 reads `webdriver.json`
# (or `WASM_BINDGEN_TEST_WEBDRIVER_JSON`) and merges `goog:chromeOptions`,
# including `binary`, into the session it creates.
#
# Prints the path of the json file.
set -euo pipefail

driver="${CHROMEDRIVER:-}"
if [ -z "$driver" ] && [ -n "${DEVENV_PROFILE:-}" ] && [ -x "$DEVENV_PROFILE/bin/chromedriver" ]; then
	driver="$DEVENV_PROFILE/bin/chromedriver"
fi
if [ -z "$driver" ]; then
	driver="$(command -v chromedriver)"
fi

version="$("$driver" --version | awk '{print $2}')"
cache="${XDG_CACHE_HOME:-$HOME/.cache}/leptoskit-chrome"
mkdir -p "$cache"

find_binary() {
	case "$(uname -s)" in
	Darwin)
		find "$cache/chrome" -type f -name "Google Chrome for Testing" -path "*${version}*" 2>/dev/null | head -1 || true
		;;
	*)
		find "$cache/chrome" -type f -name chrome -path "*${version}*" 2>/dev/null | head -1 || true
		;;
	esac
}

binary="$(find_binary)"
if [ -z "$binary" ]; then
	if ! command -v npx >/dev/null 2>&1; then
		echo "chrome_for_tests: npx is required to install Chrome ${version}" >&2
		exit 1
	fi
	# npx prints the installed path on stdout. This script's stdout is only
	# the json path: wasm-bindgen opens that path as a file.
	npx --yes @puppeteer/browsers@2 install "chrome@${version}" --path "$cache" >&2
	binary="$(find_binary)"
fi

if [ ! -x "$binary" ]; then
	echo "chrome_for_tests: Chrome ${version} did not install under ${cache}" >&2
	exit 1
fi

json="$cache/webdriver-${version}.json"
python3 - "$binary" "$json" <<'PY'
import json
import sys

binary, path = sys.argv[1], sys.argv[2]
with open(path, "w", encoding="utf-8") as out:
    json.dump({"goog:chromeOptions": {"binary": binary}}, out)
    out.write("\n")
PY
printf '%s\n' "$json"
