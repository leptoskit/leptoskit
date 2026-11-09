#!/usr/bin/env bash
# Installs Android SDK packages with sdkmanager, retrying failed downloads, and
# fails unless every package ends up installed.
#
# Hosted runners intermittently receive an archive sdkmanager cannot open
# ("Error on ZipFile unknown archive") for the NDK, the emulator, and system
# images. sdkmanager has no retry of its own, and a fresh download succeeds, so
# each attempt starts from an empty download directory. Packages already
# installed at their latest revision are not downloaded again, which is why
# installing the emulator and system image here first leaves nothing for
# android-emulator-runner's own unretried install to fetch.
#
# Usage: android_sdk_install.sh <package>...   e.g. "ndk;27.1.12297006" emulator
# ANDROID_SDK_INSTALL_ATTEMPTS (default 3) and ANDROID_SDK_RETRY_DELAY (seconds
# per attempt, default 10) tune the retry.
#
# `sdkmanager --list_installed` has two layouts. Classic tools print a pipe
# table whose path uses semicolons (`ndk;27.1.12297006`). Command-line tools
# 16111833 delegate to the Android CLI and print an indented path with
# slashes (`ndk/27.1.12297006`) and no pipes. Both name the same package.
# `setup/scripts/android_sdk_install_test.sh` feeds each layout to a fake
# sdkmanager.
set -euo pipefail

if [ "$#" -eq 0 ]; then
	echo "usage: android_sdk_install.sh <package>..." >&2
	exit 2
fi
sdk="${ANDROID_HOME:?ANDROID_HOME must name the Android SDK}"
sdkmanager="$sdk/cmdline-tools/latest/bin/sdkmanager"
attempts="${ANDROID_SDK_INSTALL_ATTEMPTS:-3}"
yes | "$sdkmanager" --licenses >/dev/null || true

missing() {
	local installed package
	# Classic rows are split on '|'. Android CLI rows are indented and use
	# '/' in the path; a package id has always used ';'.
	installed="$("$sdkmanager" --list_installed 2>/dev/null | awk '
		index($0, "|") {
			n = split($0, cols, "|")
			if (n >= 3) {
				gsub(/ /, "", cols[1])
				if (cols[1] != "" && cols[1] != "Path" && cols[1] !~ /^-+$/) print cols[1]
			}
			next
		}
		/^[[:space:]]+[^[:space:]]+/ {
			path = $1
			gsub(/\//, ";", path)
			print path
		}
	')"

	for package in "$@"; do
		grep -Fxq -- "$package" <<<"$installed" || printf '%s\n' "$package"
	done
}

for attempt in $(seq 1 "$attempts"); do
	# sdkmanager stages downloads here; never let an attempt reuse a bad one.
	rm -rf "$sdk/.temp" "$sdk/.downloadIntermediates"

	if "$sdkmanager" --install "$@" >/dev/null; then
		left="$(missing "$@")"

		if [ -z "$left" ]; then
			exit 0
		fi
		echo "sdkmanager reported success but did not install: $(paste -sd ' ' - <<<"$left")" >&2
	fi

	if [ "$attempt" -lt "$attempts" ]; then

		echo "Android SDK install attempt $attempt of $attempts failed; retrying" >&2
		sleep $((attempt * ${ANDROID_SDK_RETRY_DELAY:-10}))
	fi
done
echo "Android SDK packages were not installed after $attempts attempts: $(missing "$@" | paste -sd ' ' -)" >&2
exit 1
