//! Sending email: the [`Mailer`] trait, and a mailbox for development and
//! tests.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;

/// An email to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mail {
	/// The recipient's address.
	pub to: String,
	/// The subject line.
	pub subject: String,
	/// The plain-text body.
	pub text: String,
}

/// Why an email wasn't sent.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct MailError(pub String);

/// A future a [`Mailer`] returns.
pub type MailFuture<'a> = Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>>;

/// Sends email, however the app does it. Implement it in a few lines over
/// a provider's HTTP API:
///
/// ```ignore
/// impl Mailer for Resend {
/// 	fn send(&self, mail: Mail) -> MailFuture<'_> {
/// 		Box::pin(async move {
/// 			self.client.post(&self.url).json(&mail_json(&mail)).send().await
/// 				.map_err(|error| MailError(error.to_string()))?;
/// 			Ok(())
/// 		})
/// 	}
/// }
/// ```
pub trait Mailer: Send + Sync + 'static {
	/// Sends `mail`, returning once the provider accepted it.
	fn send(&self, mail: Mail) -> MailFuture<'_>;
}

/// Keeps the newest [`Mailbox::CAP`] emails instead of sending them: the
/// development mailbox, and the way tests read the codes they were sent.
#[derive(Clone, Debug, Default)]
pub struct Mailbox {
	messages: Arc<Mutex<Vec<Mail>>>,
}

impl Mailbox {
	/// The most emails a [`Mailbox`] keeps: past it, the oldest go, so one
	/// that lives as long as a process holds a bounded amount.
	pub const CAP: usize = 500;

	/// An empty mailbox.
	pub fn new() -> Self {
		Self::default()
	}

	/// The emails kept so far, oldest first: the newest [`Self::CAP`] sent.
	pub fn messages(&self) -> Vec<Mail> {
		self.lock().clone()
	}

	/// The newest email kept that was sent to `address`.
	pub fn latest_to(&self, address: &str) -> Option<Mail> {
		self.lock()
			.iter()
			.rev()
			.find(|mail| mail.to.eq_ignore_ascii_case(address))
			.cloned()
	}

	fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Mail>> {
		// A panic while holding the lock can't leave a `Vec` half-pushed.
		self.messages
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}
}

impl Mailer for Mailbox {
	fn send(&self, mail: Mail) -> MailFuture<'_> {
		{
			let mut messages = self.lock();
			messages.push(mail);

			if messages.len() > Self::CAP {
				let excess = messages.len() - Self::CAP;
				drop(messages.drain(..excess));
			}
		}
		Box::pin(async { Ok(()) })
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn mail(to: &str) -> Mail {
		Mail {
			to: to.to_owned(),
			subject: "Your code".to_owned(),
			text: format!("The code for {to}"),
		}
	}

	#[tokio::test]
	async fn past_its_cap_a_mailbox_keeps_the_newest_and_drops_the_oldest() {
		let mailbox = Mailbox::new();
		let sent = Mailbox::CAP + 2;

		for index in 0..sent {
			mailbox
				.send(mail(&format!("{index}@example.com")))
				.await
				.unwrap();
		}

		let kept = mailbox.messages();
		assert_eq!(kept.len(), Mailbox::CAP, "the two oldest are gone");
		assert_eq!(kept[0].to, "2@example.com");
		assert_eq!(
			kept.last().unwrap().to,
			format!("{}@example.com", sent - 1),
			"the newest is still there"
		);
		assert!(mailbox.latest_to("1@example.com").is_none());
		assert_eq!(
			mailbox.latest_to("2@example.com").unwrap().text,
			"The code for 2@example.com"
		);
	}

	#[tokio::test]
	async fn up_to_its_cap_a_mailbox_keeps_every_email() {
		let mailbox = Mailbox::new();

		for index in 0..Mailbox::CAP {
			mailbox
				.send(mail(&format!("{index}@example.com")))
				.await
				.unwrap();
		}

		let kept = mailbox.messages();
		assert_eq!(kept.len(), Mailbox::CAP);
		assert_eq!(kept[0].to, "0@example.com", "nothing was dropped");
		assert_eq!(
			kept.last().unwrap().to,
			format!("{}@example.com", Mailbox::CAP - 1)
		);
	}
}
