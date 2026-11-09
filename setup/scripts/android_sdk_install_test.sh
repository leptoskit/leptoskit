#!/usr/bin/env bash
# android_sdk_install.sh must treat a package as installed whether sdkmanager
# prints the classic pipe table (path;with;semicolons) or the current Android
# CLI table (path/with/slashes, no pipes). Command-line tools 16111833 prints
# the second form and exits 0 after a successful install; the installer used
# to report every package missing and fail.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
installer="$root/setup/scripts/android_sdk_install.sh"

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

run_install() {
	local home="$1"
	shift
	ANDROID_HOME="$home" \
		ANDROID_SDK_INSTALL_ATTEMPTS=1 \
		ANDROID_SDK_RETRY_DELAY=0 \
		bash "$installer" "$@"
}

fake_sdk() {
	local home="$1"
	local fixture="$2"
	mkdir -p "$home/cmdline-tools/latest/bin"
	cat >"$home/cmdline-tools/latest/bin/sdkmanager" <<EOF
#!/usr/bin/env bash
case "\${1:-}" in
--licenses) exit 0 ;;
--list_installed) cat "$fixture" ;;
--install) exit 0 ;;
*)
	echo "unexpected sdkmanager args: \$*" >&2
	exit 1
	;;
esac
EOF
	chmod +x "$home/cmdline-tools/latest/bin/sdkmanager"
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Classic sdkmanager --list_installed table.
cat >"$tmp/classic.txt" <<'EOF'
Installed packages:
  Path                 | Version       | Description                      | Location
  -------              | -------       | -------                          | -------
  emulator             | 37.2.13       | Android Emulator                 | emulator
  ndk;27.1.12297006    | 27.1.12297006 | NDK (Side by side) 27.1.12297006 | ndk/27.1.12297006
  platforms;android-35 | 2             | Android SDK Platform 35          | platforms/android-35
EOF

# cmdline-tools 16111833: sdkmanager is the Android CLI and prints this.
cat >"$tmp/android-cli.txt" <<'EOF'
Installed packages:
  emulator                                        37.2.13                           Android Emulator
  ndk/27.1.12297006                               27.1.12297006                     NDK (Side by side) 27.1.12297006
  platforms/android-35                            2.0.0                             Android SDK Platform 35
EOF

cat >"$tmp/missing-ndk.txt" <<'EOF'
Installed packages:
  emulator                                        37.2.13                           Android Emulator
  platforms/android-35                            2.0.0                             Android SDK Platform 35
EOF

packages=(emulator "ndk;27.1.12297006" "platforms;android-35")

fake_sdk "$tmp/classic" "$tmp/classic.txt"
if ! run_install "$tmp/classic" "${packages[@]}" >"$tmp/classic.out" 2>"$tmp/classic.err"; then
	fail "classic pipe table should count the packages as installed: $(cat "$tmp/classic.err")"
fi

fake_sdk "$tmp/cli" "$tmp/android-cli.txt"
if ! run_install "$tmp/cli" "${packages[@]}" >"$tmp/cli.out" 2>"$tmp/cli.err"; then
	fail "Android CLI slash table should count the packages as installed: $(cat "$tmp/cli.err")"
fi

fake_sdk "$tmp/partial" "$tmp/missing-ndk.txt"
if run_install "$tmp/partial" "${packages[@]}" >"$tmp/partial.out" 2>"$tmp/partial.err"; then
	fail "a package absent from the listing must fail the install"
fi
grep -q 'ndk;27.1.12297006' "$tmp/partial.err" || fail "the error must name the package that is still missing"

echo "android_sdk_install_test: ok"
