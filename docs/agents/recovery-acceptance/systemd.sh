#!/usr/bin/env bash
# Run as root in a disposable task-owned Linux VM; never on a production host.
# Input directory contains old.service, old.timer, fixed.service, fixed.timer.
set -euo pipefail
artifacts="$(realpath "${1:?artifact directory}")"
fixture="$(mktemp -d)"
unit=leptoskit-notes-production-backup
systemctl --version
mkdir -p "$fixture/old" "$fixture/fixed/etc/systemd/system"

for kind in service timer; do
	cp "$artifacts/old.$kind" "$fixture/old/$unit.$kind"

	cp "$artifacts/fixed.$kind" "$fixture/fixed/etc/systemd/system/$unit.$kind"
done

if systemd-analyze verify --man=no "$fixture/old/$unit.service" "$fixture/old/$unit.timer"; then
	echo 'FAIL: malformed negative control accepted' >&2
	exit 1
fi
systemd-analyze verify --man=no "$fixture/fixed/etc/systemd/system/$unit.service" "$fixture/fixed/etc/systemd/system/$unit.timer"
systemctl --root="$fixture/fixed" enable "$unit.timer"
systemctl --root="$fixture/fixed" is-enabled "$unit.timer"
test -L "$fixture/fixed/etc/systemd/system/timers.target.wants/$unit.timer"
cp "$fixture/fixed/etc/systemd/system/$unit."* /etc/systemd/system/
systemctl daemon-reload
test "$(systemctl show "$unit.timer" --property=Triggers --value)" = "$unit.service"
test "$(systemctl show "$unit.timer" --property=ActiveState --value)" = inactive
systemctl show "$unit.timer" --property=Triggers --property=ActiveState
sha256sum "$artifacts/"{old,fixed}.{service,timer}
echo 'PASS: negative control, fixed verification, enablement and manager linkage; no scheduled execution'
