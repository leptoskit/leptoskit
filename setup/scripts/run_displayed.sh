#!/usr/bin/env bash
# Run a command on a session bus and a 1280x1024 display.
#
# Nix's xvfb-run defaults to 640x480, which clips the GTK window the resize
# test grows to 700px. Nix dbus also has no /etc session bus config on a
# machine that did not install dbus system-wide.
#
# Without XDG_RUNTIME_DIR, AT-SPI puts its socket in ~/.cache/at-spi. Parallel
# GTK processes then share that path, and the socket is often already gone
# when a test connects. A private runtime directory gives the session one
# live socket.
set -euo pipefail

config=()
if [ ! -f /etc/dbus-1/session.conf ]; then
	config=(--config-file "$(dirname "$(command -v dbus-daemon)")/../share/dbus-1/session.conf")
fi

runtime="${XDG_RUNTIME_DIR:-}"
owned=""
if [ -z "$runtime" ] || [ ! -d "$runtime" ]; then
	runtime="$(mktemp -d)"
	chmod 700 "$runtime"
	owned="$runtime"
fi

status=0
XDG_RUNTIME_DIR="$runtime" dbus-run-session "${config[@]}" -- \
	xvfb-run -a -s "-screen 0 1280x1024x24" env GDK_BACKEND=x11 GSK_RENDERER=cairo \
	"$@" || status=$?

if [ -n "$owned" ]; then
	rm -rf "$owned"
fi
exit "$status"
