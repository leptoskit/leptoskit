//! The portable notes screen.
//!
//! It uses the types the shared crate publishes: [`Notes`] lists the newest
//! notes and [`AddNote`] adds one, on every destination and while the server
//! renders a page. The browser refreshes the list from the live socket; a
//! native app uses the HTTP client and the Refresh button.
//!
//! With sign-in, each account gets its own [`Notes`] state, and the previous
//! account's cached list is dropped before the next one is loaded.

use __CRATE___shared::AddNote;
use __CRATE___shared::Notes as NotesQuery;
use __CRATE___shared::PublicNote;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::prelude::Button;
use leptoskit::prelude::Element;
use leptoskit::prelude::For;
use leptoskit::prelude::Get;
use leptoskit::prelude::Heading;
use leptoskit::prelude::RwSignal;
use leptoskit::prelude::Set;
use leptoskit::prelude::Show;
use leptoskit::prelude::Text;
use leptoskit::prelude::TextInput;
use leptoskit::prelude::View;
use leptoskit::query::MutationHandle;
use leptoskit::query::QueryHandle;
use leptoskit::query::use_mutation;
use leptoskit::rpc::FnError;
use leptoskit::rpc::client::mutation_scope;
use leptoskit::rpc::client::query_scope;

/// The longest note the shared operation accepts, in characters.
pub const MAX_BODY: usize = 500;

/// The notes screen's state and queries. Create it in the reactive owner that
/// holds the app's [`QueryClient`](leptoskit::query::QueryClient).
#[derive(Clone)]
pub struct Notes {
	identity: String,
	list: QueryHandle<Vec<PublicNote>, FnError>,
	save: MutationHandle<AddNote, PublicNote, FnError>,
	draft: RwSignal<String>,
	problem: RwSignal<Option<String>>,
}

impl Notes {
	/// Creates the screen for `identity`, dropping any notes cached for the
	/// identity shown before it. Sign-in calls this whenever the account
	/// changes, so one person's list can't appear under another's name.
	pub fn new(identity: impl Into<String>) -> Self {
		let identity = identity.into();
		let client = leptoskit::query::use_query_client();
		client.remove_queries(leptoskit::query::QueryFilter::scope(&query_scope::<
			NotesQuery,
		>()));
		Self::build(identity)
	}

	/// Creates the screen for a host without sign-in: no account changes, so
	/// the server-rendered list can be hydrated without fetching it again.
	pub fn shared() -> Self {
		Self::build("anonymous".to_owned())
	}

	fn build(identity: String) -> Self {
		let draft = RwSignal::new(String::new());
		let save = use_mutation(
			mutation_scope::<AddNote>().with_on_success_async(move |args| {
				async move {
					// A live mutation already waits for the screen's queries to
					// show its writes; over HTTP this is
					// what brings the new note into
					// the list, and what the Refresh button does on demand.
					args.client
						.invalidate_scope(&query_scope::<NotesQuery>())
						.await;
					draft.set(String::new());
				}
			}),
		);

		Self {
			identity,
			list: list(),
			save,
			draft,
			problem: RwSignal::new(None),
		}
	}

	/// The account this screen belongs to.
	pub fn identity(&self) -> &str {
		&self.identity
	}

	/// Whether this screen still belongs to `identity`.
	pub fn owns(&self, identity: &str) -> bool {
		self.identity == identity
	}

	/// Validates the draft, then sends `AddNote`. The server checks the same
	/// length rule, so a body this accepts still can fail, and the mutation's
	/// message appears above the list.
	pub fn submit(&self) {
		match validate(&self.draft.get_untracked()) {
			Ok(body) => {
				self.problem.set(None);
				self.save.mutate(AddNote { body });
			}
			Err(problem) => self.problem.set(Some(problem)),
		}
	}

	/// Builds the screen: the form, the mutation's feedback, and the list in
	/// its loading, empty, failed, and loaded states.
	pub fn screen(&self) -> Element {
		let list = self.list;
		let save = self.save;
		let draft = self.draft;
		let problem = self.problem;
		let this = self.clone();

		View()
			.class("w-full max-w-[640px] gap-4 p-6 bg-slate-950 text-white")
			.child(Heading(1, "Notes").class("text-3xl font-bold"))
			.child(Text("Write a note. It's saved for you alone.").class("text-sm text-slate-300"))
			.child(
				View()
					.class("flex-row items-center gap-2")
					.child(
						TextInput()
							.placeholder("Write a note")
							.value(draft.get_untracked())
							.on_change(move |text| {
								draft.set(text.to_owned());
								problem.set(None);
							})
							.class("h-11 flex-1 rounded-lg px-3 bg-white text-slate-900"),
					)
					.child(
						Button("Add note")
							.enabled(!save.is_pending())
							.aria_busy(save.is_pending())
							.on_press(move || this.submit())
							.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
					),
			)
			.child(Show(
				move || problem.get().is_some(),
				move || Text(problem.get().unwrap_or_default()).class("text-sm text-rose-400"),
			))
			.child(Show(
				move || save.is_pending(),
				|| Text("Saving…").class("text-sm text-slate-300"),
			))
			.child(Show(
				move || save.is_success(),
				|| Text("Saved.").class("text-sm text-emerald-400"),
			))
			.child(Show(
				move || save.is_error(),
				move || {
					Text(save.error().map_or_else(
						|| "The note couldn't be saved.".to_owned(),
						|error| format!("The note couldn't be saved: {}", error.message()),
					))
					.class("text-sm text-rose-400")
				},
			))
			.child(
				View()
					.class("flex-row items-center gap-2")
					.child(
						Text(move || {
							match list.data() {
								Some(notes) if notes.len() == 1 => "1 note".to_owned(),
								Some(notes) => format!("{} notes", notes.len()),
								None => "Notes".to_owned(),
							}
						})
						.class("flex-1 text-sm text-slate-300"),
					)
					.child(
						Button("Refresh")
							.enabled(!list.is_fetching())
							.aria_busy(list.is_fetching())
							.on_press(move || drop(list.refetch()))
							.class("rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"),
					),
			)
			.child(Show(
				move || list.is_loading(),
				|| Text("Loading notes…").class("text-slate-300"),
			))
			.child(Show(
				move || list.is_error() && list.data().is_none(),
				move || {
					Text(list.error().map_or_else(
						|| "The notes couldn't be loaded.".to_owned(),
						|error| format!("The notes couldn't be loaded: {}", error.message()),
					))
					.class("text-sm text-rose-400")
				},
			))
			.child(Show(
				move || list.data().is_some_and(|notes| notes.is_empty()),
				|| {
					Text("No notes yet. The first one goes here.")
						.class("rounded-lg bg-slate-900 p-4 text-slate-300")
				},
			))
			.child(For(
				move || list.data().unwrap_or_default(),
				|note| note.id.to_string(),
				|note| {
					View()
						.class("rounded-lg bg-slate-900 p-4")
						.child(Text(note.body).class("text-base text-slate-100"))
				},
			))
	}
}

/// The browser keeps the list current over the live socket; a native app has
/// no live transport in the generated hosts, so it reads over HTTP.
#[cfg(feature = "web")]
fn list() -> QueryHandle<Vec<PublicNote>, FnError> {
	leptoskit::rpc::client::use_live(|| NotesQuery {})
}

#[cfg(not(feature = "web"))]
fn list() -> QueryHandle<Vec<PublicNote>, FnError> {
	leptoskit::rpc::client::use_rpc_query(|| NotesQuery {})
}

/// Checks what [`Notes::submit`] can before it sends anything: a note has
/// content, and the server's length limit counts characters.
pub fn validate(body: &str) -> Result<String, String> {
	let body = body.trim();

	if body.is_empty() {
		return Err("Write a note before adding it.".to_owned());
	}

	if body.chars().count() > MAX_BODY {
		return Err(format!("Keep the note to {MAX_BODY} characters."));
	}

	Ok(body.to_owned())
}

#[cfg(test)]
mod tests {
	use super::MAX_BODY;
	use super::validate;

	#[test]
	fn a_note_needs_something_to_say() {
		assert!(validate("").is_err());
		assert!(validate("   \n\t").is_err());
		assert_eq!(validate("  Milk  ").as_deref(), Ok("Milk"));
	}

	#[test]
	fn the_body_stops_at_the_servers_limit_in_characters() {
		let longest = "a".repeat(MAX_BODY);
		assert_eq!(validate(&longest).as_deref(), Ok(longest.as_str()));
		assert!(validate(&"a".repeat(MAX_BODY + 1)).is_err());

		// The server counts characters too, not bytes.
		let wide = "é".repeat(MAX_BODY);
		assert_eq!(validate(&wide).as_deref(), Ok(wide.as_str()));
	}
}
