#!/usr/bin/env bash
# Idempotent Linux setup for a Cursor cloud agent.
#
# Installs single-user Nix with flakes, devenv, and the public devenv and
# nix-community binary caches, then materialises this repo's shell and
# fetches its locked dependencies. macOS, iOS, Windows, Android emulators,
# and the HarmonyOS device SDK are not available on this VM.

set -euo pipefail

if [ -f /agent/repos/leptoskit/devenv.nix ]; then
	cd /agent/repos/leptoskit
elif [ ! -f devenv.nix ]; then
	echo "leptoskit install: devenv.nix not found from $(pwd)" >&2
	exit 1
fi

export CARGO_NET_RETRY="${CARGO_NET_RETRY:-10}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"

nix_conf_dir="${XDG_CONFIG_HOME:-$HOME/.config}/nix"
nix_conf="$nix_conf_dir/nix.conf"

source_nix() {
	if [ -f "$HOME/.nix-profile/etc/profile.d/nix.sh" ]; then
		# shellcheck disable=SC1091
		. "$HOME/.nix-profile/etc/profile.d/nix.sh"
	fi
}

# Append a setting only when that key is not already present, so a second
# run does not stack duplicate lines.
ensure_setting() {
	local file="$1"
	local key="$2"
	local line="$3"
	if grep -q "^${key} " "$file" 2>/dev/null; then
		return 0
	fi
	printf '%s\n' "$line" >>"$file"
}

# `cachix use` writes `substituters`, which replaces Nix's defaults instead of
# extending `extra-substituters`. Add any missing cache onto that line.
ensure_words() {
	local file="$1"
	local key="$2"
	shift 2
	local word
	if ! grep -q "^${key} " "$file" 2>/dev/null; then
		return 0
	fi
	local line
	for word in "$@"; do
		line="$(grep "^${key} " "$file" || true)"
		case "$line" in
		*"$word"*) continue ;;
		esac
		sed -i "s|^${key} \\(.*\\)|${key} \\1 ${word}|" "$file"
	done
}

configure_nix() {
	mkdir -p "$nix_conf_dir"
	touch "$nix_conf"
	ensure_setting "$nix_conf" "experimental-features" "experimental-features = nix-command flakes"
	# Used when `cachix use` has not written an explicit `substituters` line.
	ensure_setting "$nix_conf" "extra-substituters" "extra-substituters = https://devenv.cachix.org https://nix-community.cachix.org"
	ensure_setting "$nix_conf" "extra-trusted-public-keys" "extra-trusted-public-keys = devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw= nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
	ensure_words "$nix_conf" "substituters" "https://devenv.cachix.org" "https://nix-community.cachix.org"
	ensure_words "$nix_conf" "trusted-public-keys" \
		"devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=" \
		"nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="

	# The same file for commands that do not see the user config directory.
	if sudo -n true 2>/dev/null; then
		sudo mkdir -p /etc/nix
		sudo cp "$nix_conf" /etc/nix/nix.conf
	fi
}

link_tool() {
	local name="$1"
	local bin
	bin="$(command -v "$name")"
	sudo ln -sfn "$bin" "/usr/local/bin/$name"
}

install_nix() {
	if [ -x "$HOME/.nix-profile/bin/nix" ] || command -v nix >/dev/null 2>&1; then
		source_nix
		return 0
	fi
	curl --retry 5 --retry-all-errors --retry-delay 2 -fsSL https://nixos.org/nix/install | sh -s -- --no-daemon
	source_nix
}

install_profile_tool() {
	local attr="$1"
	local name="$2"
	if command -v "$name" >/dev/null 2>&1; then
		return 0
	fi
	local attempt
	for attempt in 1 2 3; do
		if nix profile install --accept-flake-config "nixpkgs#$attr"; then
			hash -r
			return 0
		fi
		echo "nix profile install nixpkgs#$attr attempt $attempt failed; retrying in 20s" >&2
		if [ "$attempt" -eq 3 ]; then
			return 1
		fi
		sleep 20
	done
}

install_nix
configure_nix

# Login shells source /etc/profile.d. Non-login shells find the binaries
# through the /usr/local/bin links below; Nix still reads nix.conf from $HOME.
if sudo -n true 2>/dev/null; then
	sudo tee /etc/profile.d/nix-devenv.sh >/dev/null <<'EOF'
if [ -f "${HOME}/.nix-profile/etc/profile.d/nix.sh" ]; then
	. "${HOME}/.nix-profile/etc/profile.d/nix.sh"
fi
EOF
fi

install_profile_tool devenv devenv
install_profile_tool cachix cachix

# Public cache. Idempotent: a second run leaves the existing nix.conf lines.
cachix use devenv
configure_nix

link_tool nix
link_tool devenv
link_tool cachix

# Build the pinned shell (GTK, rustup, linters, drivers) and fetch crates.
# The toolchain in rust-toolchain.toml installs on the first rustc call.
devenv shell -- true
devenv shell -- install:all
