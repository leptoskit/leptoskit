//! `/device`: approve a terminal (`docs/agents/cloud-design.md` §7.4).
//!
//! The person who ran `lk cloud login` compares the code their terminal
//! shows with the one here, and approves it. Approving someone else's code
//! by accident should be hard: the sheet says what the terminal will be able
//! to do, as whom, and to approve only a code you started yourself.

use leptoskit::auth::ErrorResponse;
use leptoskit::idp::DeviceDecision;
use leptoskit::idp::DeviceRequest;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit_router::Routable as _;

use super::masthead;
use super::redirect;
use crate::components::CommandBlock;
use crate::components::Kind;
use crate::components::Stamp;
use crate::components::StampSize;
use crate::components::Tone;
use crate::components::block_button;
use crate::components::field;
use crate::components::focus_ring;
use crate::components::link;
use crate::components::margin_note;
use crate::components::plain_button;
use crate::components::problem;
use crate::components::raised;
use crate::components::sheet;
use crate::components::sheet_rule;
use crate::console::Console;
use crate::console::Load;
use crate::console::Who;
use crate::console::first_organization;
use crate::console::looks_new;
use crate::console::normalize_code;
use crate::console::spell_code;
use crate::routes::Device;
use crate::routes::Navigation;
use crate::routes::Route;
use crate::style;

/// The device sheet's states.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceSheet {
	/// No code in the address: ask for it.
	Enter {
		/// What's wrong with what was typed.
		problem: Option<String>,
	},
	/// The code is being looked up.
	Loading,
	/// No terminal is waiting for this code.
	Expired {
		/// The code.
		code: String,
	},
	/// The lookup failed for another reason.
	Failed {
		/// The code.
		code: String,
		/// What went wrong.
		error: String,
	},
	/// A terminal is waiting: approve or deny it.
	Ask(Box<Ask>),
	/// Approved.
	Approved {
		/// The organisation to go on to.
		organization: Option<String>,
		/// Whether the person has shipped before.
		has_projects: bool,
	},
	/// Denied.
	Denied,
}

/// A terminal waiting for approval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
	/// What the terminal asks for.
	pub request: DeviceRequest,
	/// Who it will act as.
	pub person: String,
	/// The account was just made.
	pub new_account: bool,
	/// The decision is being sent.
	pub deciding: Option<bool>,
	/// The decision failed.
	pub problem: Option<String>,
}

/// What to say when the device flow fails, from the server's error.
#[must_use]
pub fn device_problem(error: &ErrorResponse) -> String {
	match error.error.as_str() {
		"rate_limited" => "Too many tries. Wait a minute, then try again.".to_owned(),
		"not_found" => {
			"This code has expired or was already used. Run “lk cloud login” again for a new one."
				.to_owned()
		}
		_ => error.message.clone(),
	}
}

impl DeviceSheet {
	/// The sheet the console's state calls for on `route`.
	#[must_use]
	pub fn read(console: &Console, route: &Device) -> Self {
		let flow = console.device;
		let code = route
			.user_code
			.as_deref()
			.map(normalize_code)
			.filter(|code| !code.is_empty());
		let Some(code) = code else {
			return Self::Enter { problem: None };
		};

		if let Some(decision) = flow.decide.variables()
			&& flow.decide.is_success()
			&& decision.user_code == code
		{
			if !decision.approve {
				return Self::Denied;
			}

			let (organization, has_projects) = match console.me() {
				Load::Ready(me) => {
					(
						first_organization(&me).map(|org| org.slug.clone()),
						console
							.projects
							.data()
							.is_some_and(|projects| !projects.is_empty()),
					)
				}
				_ => (None, false),
			};

			return Self::Approved {
				organization,
				has_projects,
			};
		}

		match (flow.request.data(), flow.request.error()) {
			(Some(request), _) => {
				let me = console.me();
				let person = match (&me, console.who()) {
					(Load::Ready(me), _) if me.email.is_some() => {
						me.email.clone().unwrap_or_default()
					}
					_ => "you".to_owned(),
				};
				let new_account = match &me {
					Load::Ready(me) => looks_new(me, console.projects.data().as_deref()),
					_ => false,
				};
				let decision_error = flow.decide.error();

				if decision_error
					.as_ref()
					.is_some_and(|error| error.error == "not_found")
				{
					return Self::Expired { code };
				}

				Self::Ask(Box::new(Ask {
					request,
					person,
					new_account,
					deciding: flow
						.decide
						.is_pending()
						.then(|| flow.decide.variables().is_some_and(|d| d.approve)),
					problem: decision_error.map(|error| device_problem(&error)),
				}))
			}
			(None, Some(error)) if error.error == "not_found" => Self::Expired { code },
			(None, Some(error)) => {
				Self::Failed {
					code,
					error: device_problem(&error),
				}
			}
			(None, None) => Self::Loading,
		}
	}
}

/// `/device`.
pub fn page(route: &Device, navigation: &Navigation) -> Element {
	let console = Console::get();
	let here = Route::Device(route.clone()).href().to_string();

	match console.who() {
		Who::SignedOut => {
			redirect(navigation, &super::sign_in_href(&here));
			return super::signed_out(&here);
		}
		Who::Checking => return super::loading("Loading your session", &["h-80 w-full max-w-md"]),
		Who::Unknown(error) => return super::session_failed(console, &error),
		Who::SignedIn(_) => {}
	}

	view(&console, navigation, &DeviceSheet::read(&console, route))
}

/// The device sheet in `state`.
pub fn view(console: &Console, navigation: &Navigation, state: &DeviceSheet) -> Element {
	let body: Vec<Element> = match state {
		DeviceSheet::Enter { problem } => enter(console, navigation, problem.as_deref()),
		DeviceSheet::Loading => {
			vec![super::loading(
				"Looking up the code",
				&["h-10 w-48", "h-24 w-full"],
			)]
		}
		DeviceSheet::Expired { code } => {
			vec![
				Heading(1, "This code can't be used.").class(style::CARD_TITLE),
				code_text(code),
				problem(
					"This code has expired or was already used. Run “lk cloud login” again for a \
					 new one.",
				),
				link("Enter a different code →", "/device"),
			]
		}
		DeviceSheet::Failed { code, error } => {
			let request = console.device.request;
			vec![
				Heading(1, "Does your terminal show this code?").class(style::CARD_TITLE),
				code_text(code),
				super::load_failed("this code", error, move || {
					drop(request.refetch());
				}),
			]
		}
		DeviceSheet::Ask(ask) => self::ask(console, ask),
		DeviceSheet::Approved {
			organization,
			has_projects,
		} => approved(console, organization.as_deref(), *has_projects),
		DeviceSheet::Denied => denied(),
	};

	sheet(
		[
			masthead(),
			Text("CONNECT A TERMINAL").class(style::GROUP_LABEL),
		]
		.into_iter()
		.chain(body),
	)
	.test_id("device")
}

/// The code, large, spelled out for a screen reader.
fn code_text(code: &str) -> Element {
	Text(code.to_owned())
		.class("font-[var(--cloud-font-mono)] text-3xl font-semibold tracking-wider self-center")
		.accessibility_label(spell_code(code))
		.test_id("device-code")
}

fn enter(console: &Console, navigation: &Navigation, problem_text: Option<&str>) -> Vec<Element> {
	let flow = console.device;
	let typing = console.typing;
	let go = {
		let navigation = navigation.clone();
		move || {
			let code = normalize_code(&flow.typed.get_untracked());

			if code.is_empty() {
				return;
			}

			navigation.navigate(Device {
				user_code: Some(code),
			});
		}
	};
	let input = TextInput()
		.value(flow.typed.get())
		.placeholder("WDJB-MJHT")
		.autocomplete("one-time-code")
		.focused(true)
		.on_change(move |text: &str| flow.typed.set(text.to_owned()))
		.on(NativeEventSlot::Focus, move |_| {
			typing.set(true);
			EventOutcome::Unhandled
		})
		.on(NativeEventSlot::Blur, move |_| {
			typing.set(false);
			EventOutcome::Unhandled
		})
		.on_key({
			let go = go.clone();
			move |key| {
				if key.is("Enter", false, false, false) {
					go();
					return EventOutcome::Handled;
				}

				EventOutcome::Unhandled
			}
		});

	vec![
		Heading(1, "Enter the code your terminal shows.").class(style::CARD_TITLE),
		field(
			"device-code-input",
			"Code from your terminal",
			input,
			problem_text,
		),
		focus_ring(
			console.focus,
			"device-continue",
			block_button("Continue", None, go),
		)
		.class("w-full"),
	]
}

fn ask(console: &Console, ask: &Ask) -> Vec<Element> {
	let flow = console.device;
	let code = ask.request.user_code.clone();
	let decide = move |approve: bool| {
		let user_code = code.clone();
		move || {
			flow.decide.mutate(DeviceDecision {
				user_code: user_code.clone(),
				approve,
			});
		}
	};
	let mut lines = Vec::new();

	if ask.new_account {
		lines.push(margin_note("Your account is ready."));
	}

	lines.push(Heading(1, "Does your terminal show this code?").class(style::CARD_TITLE));
	lines.push(code_text(&ask.request.user_code));
	lines.push(sheet_rule());
	lines.push(
		Text(format!(
			"{} asks to act as {}:",
			ask.request.app.name, ask.person
		))
		.class(style::BODY),
	);
	lines.push(
		View()
			.class("flex-col gap-1.5 pl-1")
			.role(Role::List)
			.accessibility_label("What it can do")
			.children(ask.request.scopes.iter().map(|scope| {
				View()
					.class("flex-row items-start gap-2")
					.role(Role::ListItem)
					.child(Text("•").class(style::SECONDARY).aria_hidden(true))
					// Takes the row's width and wraps, rather than running
					// past the sheet's edge.
					.child(
						Text(scope.description.clone())
							.class(style::SECONDARY)
							.class("flex-1"),
					)
			})),
	);
	lines.push(sheet_rule());
	lines.push(Text("Only approve a code you started yourself, just now.").class(style::BODY));

	if let Some(problem_text) = &ask.problem {
		lines.push(problem(problem_text.clone()));
	}

	let approving = ask.deciding == Some(true);
	let denying = ask.deciding == Some(false);
	let mut deny = plain_button(
		Kind::Quiet,
		"Not me, deny",
		denying.then_some("Denying…"),
		decide(false),
	);

	if approving {
		deny = deny.enabled(false);
	}

	let mut approve = crate::components::button(
		Kind::Primary,
		"Approve this terminal",
		approving.then_some("Approving…"),
		decide(true),
	);

	if denying {
		approve = approve.enabled(false);
	}

	lines.push(
		View()
			.class("flex-row flex-wrap items-center gap-3")
			.child(focus_ring(console.focus, "device-approve", approve))
			.child(focus_ring(console.focus, "device-deny", deny)),
	);

	lines
}

fn approved(console: &Console, organization: Option<&str>, has_projects: bool) -> Vec<Element> {
	let mut lines = vec![
		Stamp {
			words: "APPROVED".to_owned(),
			label: "Approved".to_owned(),
			tone: Tone::Good,
			seed: "device".to_owned(),
			size: StampSize::Sheet,
		}
		.build(),
		Heading(1, "Your terminal is signed in.").class(style::CARD_TITLE),
		Text("Next, in the same terminal:").class(style::BODY),
		CommandBlock::one("lk cloud launch").build(console.copied),
	];

	if has_projects {
		lines.push(
			Text("or “lk deploy” in a project you've launched before.").class(style::SECONDARY),
		);
	}

	let next = organization.map_or_else(|| "/organizations".to_owned(), |org| format!("/{org}"));
	// Focused, so Enter is enough.
	lines.push(focus_ring(
		console.focus,
		"device-next",
		raised(
			Pressable("Continue")
				.href(next)
				.class(Kind::Primary.class_name())
				.focused(true)
				.child(Text("Continue")),
		),
	));

	lines
}

fn denied() -> Vec<Element> {
	vec![
		Stamp {
			words: "DENIED".to_owned(),
			label: "Denied".to_owned(),
			tone: Tone::Bad,
			seed: "device".to_owned(),
			size: StampSize::Sheet,
		}
		.build(),
		Heading(1, "Denied.").class(style::CARD_TITLE),
		Text(
			"Your terminal will say so. If you didn't start this, someone may have sent you the \
			 link; you don't need to do anything else.",
		)
		.class(style::BODY),
	]
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn expired_and_used_codes_read_the_same() {
		let error = ErrorResponse {
			error: "not_found".to_owned(),
			message: "no such code".to_owned(),
			retry_after: None,
		};
		assert_eq!(
			device_problem(&error),
			"This code has expired or was already used. Run “lk cloud login” again for a new one."
		);
	}
}
