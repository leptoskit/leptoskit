//! Keep directives on literal lines: a split escaped newline can turn
//! `ExecStart` into part of the previous directive's value.

pub(super) fn service(name: &str, base: &str) -> String {
	format!(
		r"[Unit]
Description=Encrypted verified database backup for {name}
After=docker.service network-online.target
Wants=network-online.target
[Service]
Type=oneshot
UMask=0077
ExecStart=/usr/bin/python3 {base}/recovery.py backup
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths={base} /run/docker.sock
"
	)
}

pub(super) fn timer(name: &str, schedule: &str) -> String {
	format!(
		r"[Unit]
Description=Database backup schedule for {name}
[Timer]
OnCalendar={schedule}
Persistent=true
RandomizedDelaySec=300
[Install]
WantedBy=timers.target
"
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn directive<'a>(text: &'a str, section: &str, key: &str) -> &'a str {
		let mut current = "";
		let mut value = None;
		for line in text.lines() {
			assert!(!line.ends_with('\\'), "unexpected continuation: {line}");
			if line.starts_with('[') {
				assert!(line.ends_with(']'), "invalid section: {line}");
				current = line;
			} else if let Some((name, candidate)) = line.split_once('=') {
				if current == section && name == key {
					assert!(value.is_none(), "duplicate {key}");
					value = Some(candidate);
				}
			} else {
				panic!("invalid directive: {line}");
			}
		}
		value.unwrap_or_else(|| panic!("missing {section} {key}"))
	}

	#[test]
	fn service_executes_the_worker_with_its_writable_state_directory() {
		let unit = service("notes", "/var/lib/leptoskit/notes/control");
		assert_eq!(directive(&unit, "[Service]", "UMask"), "0077");
		assert_eq!(
			directive(&unit, "[Service]", "ExecStart"),
			"/usr/bin/python3 /var/lib/leptoskit/notes/control/recovery.py backup"
		);
		assert_eq!(directive(&unit, "[Service]", "ProtectSystem"), "strict");
		assert_eq!(
			directive(&unit, "[Service]", "ReadWritePaths"),
			"/var/lib/leptoskit/notes/control /run/docker.sock"
		);
	}

	#[test]
	fn timer_is_installable_with_the_configured_calendar() {
		let unit = timer("notes", "*-*-* 03:00:00");
		assert_eq!(directive(&unit, "[Timer]", "OnCalendar"), "*-*-* 03:00:00");
		assert_eq!(directive(&unit, "[Timer]", "Persistent"), "true");
		assert_eq!(directive(&unit, "[Install]", "WantedBy"), "timers.target");
	}
}
