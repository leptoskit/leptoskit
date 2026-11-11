#!/usr/bin/env bash
# Nix pins chromedriver. GitHub's Chrome moves on its own, and a driver
# refuses to start a Chrome whose major version is not its own. Install
# Chrome for Testing at the driver's major version and point wasm-bindgen
# at that binary.
set -euo pipefail

driver="${CHROMEDRIVER:-}"
if [[ ! -x "$driver" ]]; then
	driver="$(command -v chromedriver)"
fi

version="$("$driver" --version | awk '{print $2}')"
major="${version%%.*}"
dest="${RUNNER_TEMP:-/tmp}/chrome-for-testing"
npx --yes @puppeteer/browsers@2 install "chrome@${major}" --path "$dest"
bin="$(find "$dest" -type f -name chrome -print -quit)"
[[ -n "$bin" && -x "$bin" ]]

config="${RUNNER_TEMP:-/tmp}/webdriver-chrome.json"
cat >"$config" <<EOF
{
  "goog:chromeOptions": {
    "binary": "${bin}",
    "args": [
      "--headless",
      "--no-sandbox",
      "--disable-gpu",
      "--mute-audio",
      "--autoplay-policy=no-user-gesture-required"
    ]
  }
}
EOF

if [[ -n "${GITHUB_ENV:-}" ]]; then
	echo "WASM_BINDGEN_TEST_WEBDRIVER_JSON=${config}" >>"$GITHUB_ENV"
fi
echo "chromedriver ${version} will launch ${bin}"
