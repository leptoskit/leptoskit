//! The address a phone scans to open the app `leptoskit dev` is serving.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::UdpSocket;

use leptoskit_plugin_dev_client::dev_ip;
use leptoskit_plugin_dev_client::session_link;

use crate::ui::DIM;

/// The private address this machine would use to reach another local network,
/// when it has one.
pub fn lan_address() -> Option<IpAddr> {
	let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
	// Documentation range. UDP connect chooses a source address and sends
	// nothing.
	socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
	let address = socket.local_addr().ok()?.ip();
	dev_ip(address).then_some(address)
}

/// Unicode QR art for `text`, or `None` when it does not fit in one code.
pub fn qr_text(text: &str) -> Option<String> {
	let code = qrcode::QrCode::new(text).ok()?;
	Some(
		code.render::<qrcode::render::unicode::Dense1x2>()
			.quiet_zone(true)
			.build(),
	)
}

/// Prints how a phone reaches the development guests on `port`.
pub fn print_session(port: u16) {
	let lan = lan_address().and_then(|address| session_link(address, port));
	anstream::println!();
	anstream::println!("{DIM}Open the LeptosKit app and enter this address, or scan it.{DIM:#}");
	print_lan(lan.as_deref());
	anstream::println!("{DIM}This computer:{DIM:#} http://127.0.0.1:{port}/_leptoskit/dev");
	anstream::println!("{DIM}Android emulator:{DIM:#} http://10.0.2.2:{port}/_leptoskit/dev");
	anstream::println!(
		"{DIM}The guest is unsigned. Only a debug build of the LeptosKit app will run it.{DIM:#}"
	);
}

fn print_lan(link: Option<&str>) {
	let Some(link) = link else {
		anstream::println!(
			"{DIM}This machine has no private network address, so a phone on Wi-Fi cannot reach \
			 it.{DIM:#}"
		);
		return;
	};
	anstream::println!("{link}");

	if let Some(code) = qr_text(link) {
		anstream::println!("{code}");
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_code_encodes_a_local_session_and_refuses_a_huge_one() {
		let link = "http://192.168.1.20:43123/_leptoskit/dev";
		let code = qr_text(link).unwrap();
		assert!(code.lines().count() > 4, "{code}");
		assert!(
			code.chars()
				.any(|mark| matches!(mark, '█' | '▀' | '▄' | ' '))
		);
		assert!(qr_text(&"x".repeat(8_000)).is_none());
	}

	#[test]
	fn a_lan_lookup_does_not_panic() {
		let _ = lan_address();
	}
}
