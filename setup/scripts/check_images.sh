#!/usr/bin/env bash
# The repository hosts images instead of committing them (docs/agents/images.md):
# they land on Backblaze or a GitHub attachment, and the Markdown links the URL.
# This check fails when a tracked file has an image or video extension that is
# not in .github/allowed-images.txt, when an allowlisted file outgrows the
# 16 KiB limit without the `!` scaffolding exemption, and when the allowlist
# no longer matches the tree.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

allowlist=".github/allowed-images.txt"
limit=$((16 * 1024))
pattern='\.(png|jpe?g|gif|webp|avif|svg|ico|bmp|tiff?|mp4|webm|mov)$'

hosting_advice() {
	cat <<'EOF'
Images are not committed to this repository. Host the image and link the URL:
  - Backblaze B2, bucket `leptoskit`:
      nix run nixpkgs#backblaze-b2 -- upload-file --no-progress leptoskit <file> <remote-path>
    and link https://f002.backblazeb2.com/file/leptoskit/<remote-path>.
  - Or attach it to a GitHub issue or pull request:
      gh pr comment --attach <file>   (or: gh issue comment --attach <file>)
    and link the returned https://github.com/user-attachments/... URL.
See docs/agents/images.md. Only fixtures, brand vectors, and toolchain
scaffolding belong in .github/allowed-images.txt.
EOF
}

if [ ! -f "$allowlist" ]; then
	echo "error: $allowlist is missing" >&2
	hosting_advice
	exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The allowlist holds one git path per line; `!` exempts it from the size
# limit, and comment lines carry the reasons.
awk '/^#/ || /^$/ {next} {path = $0; sub(/^!/, "", path); print path}' "$allowlist" |
	sort >"$work/allowed"

# `|| true` keeps an empty tree from failing the pipeline; an empty tracked
# list is a valid state the comparisons below handle.
git ls-files | grep -iE "$pattern" | sort >"$work/tracked" || true

# Duplicate entries hide which reason belongs to which file.
awk '/^#/ || /^$/ {next} {path = $0; sub(/^!/, "", path); count[path]++}
	END {for (path in count) if (count[path] > 1) print path}' "$allowlist" >"$work/duplicates"

if [ -s "$work/duplicates" ]; then
	while IFS= read -r path; do
		echo "error: $allowlist lists $path more than once" >&2
	done <"$work/duplicates"
	exit 1
fi

unlisted="$(comm -23 "$work/tracked" "$work/allowed")"
if [ -n "$unlisted" ]; then
	while IFS= read -r path; do
		echo "error: $path is tracked but not allowlisted" >&2
	done <<<"$unlisted"
	hosting_advice
	exit 1
fi

stale="$(comm -13 "$work/tracked" "$work/allowed")"
if [ -n "$stale" ]; then
	while IFS= read -r path; do
		echo "error: $allowlist lists $path, which git does not track; remove the line" >&2
	done <<<"$stale"
	exit 1
fi

oversized=0
while IFS= read -r entry; do
	path="$entry"
	exempt=0
	if [ "${entry:0:1}" = "!" ]; then
		path="${entry:1}"
		exempt=1
	fi

	[ -f "$path" ] || continue

	bytes="$(wc -c <"$path")"
	if [ "$bytes" -gt "$limit" ] && [ "$exempt" -eq 0 ]; then
		echo "error: $path is allowlisted but $bytes bytes exceeds the ${limit}-byte limit" >&2
		oversized=1
	fi
done <"$allowlist"

if [ "$oversized" -ne 0 ]; then
	echo "Host the image instead, or if it is platform toolchain scaffolding the" >&2
	echo "toolchain requires, prefix its line in $allowlist with \`!\`." >&2
	exit 1
fi

echo "images: every tracked image is allowlisted and within its size limit"
