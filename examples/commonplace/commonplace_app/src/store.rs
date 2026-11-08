//! The app's state, and the actions that change it.
//!
//! Screens read [`State`] through [`Store::current`], a whole field at a
//! time: each field is its own signal, so a screen that reads one through its
//! accessor renders again only when that field changes. Actions that need
//! the server run as tasks: they mark what they are loading, call the API,
//! and write the answer back to the field, and the screen renders again.

use std::cell::RefCell;

use commonplace_shared::ClipChanges;
use commonplace_shared::ClipId;
use commonplace_shared::NewClipInput;
use commonplace_shared::NewProjectInput;
use commonplace_shared::NoteId;
use commonplace_shared::PageInput;
use commonplace_shared::ProjectDetail;
use commonplace_shared::ProjectId;
use commonplace_shared::ProjectSummary;
use commonplace_shared::TextQuote;
use commonplace_shared::cite::Style;
use leptoskit::prelude::GetUntracked;
use leptoskit::prelude::Set;
use leptoskit::prelude::Update;
use leptoskit::prelude::With;
use leptoskit::prelude::WithUntracked;
use leptoskit::store::ArcField;
use leptoskit::store::ArcStore;

use crate::api::Api;
use crate::api::Problem;
use crate::tasks::spawn;

/// Something loaded from the server.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub enum Load<T> {
	#[default]
	Idle,
	Loading,
	Ready(T),
	Failed(String),
}

impl<T> Load<T> {
	/// The loaded value, if there is one.
	pub fn ready(&self) -> Option<&T> {
		match self {
			Self::Ready(value) => Some(value),
			_ => None,
		}
	}
}

/// Whether someone is signed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub enum Session {
	/// Still asking the server.
	#[default]
	Checking,
	SignedOut,
	SignedIn,
}

/// The sign-in form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub struct SignInForm {
	pub email: String,
	pub code: String,
	/// The code has been sent, so the form asks for it.
	pub sent: bool,
	pub busy: bool,
	pub error: Option<String>,
}

/// A clip being written by hand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub struct ClipDraft {
	pub url: String,
	pub title: String,
	pub author: String,
	pub site: String,
	pub published: String,
	pub quote: String,
	pub note: String,
	pub tags: String,
}

/// Everything the screens show.
#[derive(Clone, Debug, Default, PartialEq, reactive_stores::Store)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub struct State {
	pub session: Session,
	pub sign_in: SignInForm,
	pub library: Load<Vec<ProjectSummary>>,
	/// The open project.
	pub project: Option<(ProjectId, Load<ProjectDetail>)>,
	pub new_project: NewProjectInput,
	pub clip: ClipDraft,
	pub note: String,
	/// A clip's note being edited, with its text so far.
	pub editing: Option<(ClipId, String)>,
	pub style: Style,
	/// Something went wrong that isn't one form's error.
	pub notice: Option<String>,
	/// An action is waiting for the server.
	pub busy: bool,
	/// The extension's own state.
	pub extension: ExtensionState,
}

/// What the extension's surfaces add.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "devtools", derive(serde::Serialize, serde::Deserialize))]
pub struct ExtensionState {
	/// The page the popup, or a content script's frame, clips from.
	pub page: Option<PageInput>,
	/// The selection a content script's frame clips.
	pub quote: Option<TextQuote>,
	/// The project clips go to.
	pub project: Option<ProjectId>,
	/// What happened to the last clip, in words.
	pub status: Option<String>,
	/// The sites that show highlights, as match patterns.
	pub sites: Vec<String>,
	/// The site being added in the settings.
	pub site: String,
	/// The clip whose highlight was pressed.
	pub pressed: Option<commonplace_shared::ClipOnPage>,
	/// The clips taken from the page, by their highlight's ID.
	pub on_page: Vec<commonplace_shared::ClipOnPage>,
}

thread_local! {
	static STORE: Store = Store::default();
	static API: RefCell<Option<Api>> = const { RefCell::new(None) };
}

/// The API client the actions use. Each host installs its own at start.
///
/// # Panics
///
/// When no client is installed.
#[must_use]
pub fn api() -> Api {
	API.with_borrow(|api| {
		api.clone()
			.expect("Commonplace installs an API client at start")
	})
}

/// Installs the API client the actions use.
pub fn install(client: Api) {
	API.with_borrow_mut(|api| *api = Some(client));
}

/// A handle on the state. It is cheap to clone, and every clone is the same
/// state.
#[derive(Clone, Default)]
pub struct Store {
	state: ArcStore<State>,
}

impl Store {
	/// The app's store.
	#[must_use]
	pub fn current() -> Self {
		STORE.with(Clone::clone)
	}

	/// Reads the state, so the screen renders again when any field changes.
	/// A screen that reads one field, through its accessor below, renders
	/// again only when that field changes.
	pub fn with<T>(&self, read: impl FnOnce(&State) -> T) -> T {
		self.state.with(read)
	}

	/// Reads the state without subscribing to it.
	pub fn peek<T>(&self, read: impl FnOnce(&State) -> T) -> T {
		self.state.with_untracked(read)
	}

	/// A snapshot, for tests.
	#[must_use]
	pub fn snapshot(&self) -> State {
		self.state.get_untracked()
	}

	/// Shows the state in `DevTools` as `store`, where it can be read and
	/// edited.
	#[cfg(feature = "devtools")]
	pub fn expose(&self) {
		leptoskit::devtools::expose("store", self.state.clone());
	}

	/// Whether someone is signed in.
	pub fn session(&self) -> ArcField<Session> {
		self.state.clone().session().into()
	}

	/// The sign-in form.
	pub fn sign_in(&self) -> ArcField<SignInForm> {
		self.state.clone().sign_in().into()
	}

	/// The library's projects, loaded from the server.
	pub fn library(&self) -> ArcField<Load<Vec<ProjectSummary>>> {
		self.state.clone().library().into()
	}

	/// The open project.
	pub fn project(&self) -> ArcField<Option<(ProjectId, Load<ProjectDetail>)>> {
		self.state.clone().project().into()
	}

	/// The new-project form.
	pub fn new_project(&self) -> ArcField<NewProjectInput> {
		self.state.clone().new_project().into()
	}

	/// A clip being written by hand.
	pub fn clip(&self) -> ArcField<ClipDraft> {
		self.state.clone().clip().into()
	}

	/// A clip's note being added to the open project.
	pub fn note(&self) -> ArcField<String> {
		self.state.clone().note().into()
	}

	/// A clip's note being edited, with its text so far.
	pub fn editing(&self) -> ArcField<Option<(ClipId, String)>> {
		self.state.clone().editing().into()
	}

	/// The citation style exports use.
	pub fn style(&self) -> ArcField<Style> {
		self.state.clone().style().into()
	}

	/// Something went wrong that isn't one form's error.
	pub fn notice(&self) -> ArcField<Option<String>> {
		self.state.clone().notice().into()
	}

	/// An action is waiting for the server.
	pub fn busy(&self) -> ArcField<bool> {
		self.state.clone().busy().into()
	}

	/// The extension's own state.
	pub fn extension(&self) -> ArcField<ExtensionState> {
		self.state.clone().extension().into()
	}

	/// Puts a failure on screen. A signed-out failure returns to sign-in.
	fn failed(&self, problem: &Problem) {
		self.busy().set(false);

		if problem.signed_out {
			self.session().set(Session::SignedOut);
		} else {
			self.notice().set(Some(problem.message.clone()));
		}
	}

	/// Finds out whether someone is signed in, then loads the library.
	pub fn start(&self, token: Option<String>) {
		let store = self.clone();
		spawn(async move {
			let api = api();
			if api.uses_bearer() && token.is_none() {
				store.session().set(Session::SignedOut);
				return;
			}

			api.set_session(token);
			match api.signed_in().await {
				Ok(true) => {
					store.session().set(Session::SignedIn);
					store.load_library();
				}
				Ok(false) => store.session().set(Session::SignedOut),
				Err(problem) => {
					store.session().set(Session::SignedOut);
					store
						.sign_in()
						.update(|form| form.error = Some(problem.message));
				}
			}
		});
	}

	/// Sends a sign-in code to the address in the form.
	pub fn send_code(&self) {
		let email = self.peek(|state| state.sign_in.email.trim().to_owned());

		if email.is_empty() {
			self.sign_in()
				.update(|form| form.error = Some("Enter your email address.".into()));

			return;
		}

		self.sign_in().update(|form| {
			form.busy = true;
			form.error = None;
		});
		let store = self.clone();
		spawn(async move {
			let result = api().send_code(&email).await;
			store.sign_in().update(|form| {
				form.busy = false;
				match result {
					Ok(()) => form.sent = true,
					Err(problem) => form.error = Some(problem.message),
				}
			});
		});
	}

	/// Signs in with the code in the form.
	pub fn verify_code(&self) {
		let (email, code) = self.peek(|state| {
			(
				state.sign_in.email.trim().to_owned(),
				state.sign_in.code.trim().to_owned(),
			)
		});
		self.sign_in().update(|form| {
			form.busy = true;
			form.error = None;
		});
		let store = self.clone();
		spawn(async move {
			match api().verify_code(&email, &code).await {
				Ok(token) => {
					crate::session::keep(token).await;
					store.sign_in().set(SignInForm::default());
					store.session().set(Session::SignedIn);
					store.load_library();
				}
				Err(problem) => {
					store.sign_in().update(|form| {
						form.busy = false;
						form.error = Some(problem.message);
					});
				}
			}
		});
	}

	/// Signs out, here and on the server. Each field is written rather than
	/// the state replaced, so a screen reading one field hears its change.
	pub fn sign_out(&self) {
		let store = self.clone();
		spawn(async move {
			let _ = api().sign_out().await;
			crate::session::keep(None).await;
			store.session().set(Session::SignedOut);
			store.sign_in().set(SignInForm::default());
			store.library().set(Load::Idle);
			store.project().set(None);
			store.new_project().set(NewProjectInput::default());
			store.clip().set(ClipDraft::default());
			store.note().set(String::new());
			store.editing().set(None);
			store.style().set(Style::default());
			store.notice().set(None);
			store.busy().set(false);
			store.extension().set(ExtensionState::default());
		});
	}

	/// Loads the library again.
	pub fn load_library(&self) {
		self.library().update(|library| {
			if library.ready().is_none() {
				*library = Load::Loading;
			}
		});
		let store = self.clone();
		spawn(async move {
			match api().projects().await {
				Ok(projects) => store.library().set(Load::Ready(projects)),
				Err(problem) if problem.signed_out => store.failed(&problem),
				Err(problem) => store.library().set(Load::Failed(problem.message)),
			}
		});
	}

	/// Opens a project, loading it unless it is already open. Screens call
	/// this while rendering, so it only starts work that isn't under way.
	pub fn open_project(&self, id: ProjectId) {
		let open = self.peek(|state| matches!(&state.project, Some((open, _)) if *open == id));

		if open {
			return;
		}

		self.reload_project(id);
	}

	/// Loads the open project again.
	pub fn reload_project(&self, id: ProjectId) {
		let store = self.clone();
		// Writing state while a screen renders would render it again in the
		// middle; the task starts after this render.
		spawn(async move {
			let keep = store.project().with_untracked(
				|project| matches!(project, Some((open, Load::Ready(_))) if *open == id),
			);
			if !keep {
				store.project().set(Some((id, Load::Loading)));
			}

			let result = api().project(id).await;
			store.project().set(Some((
				id,
				match result {
					Ok(detail) => Load::Ready(detail),
					Err(problem) => Load::Failed(problem.message),
				},
			)));
		});
	}

	/// Creates a project from the form, then opens it with `then`.
	pub fn create_project(&self, then: impl FnOnce(ProjectId) + 'static) {
		let input = self.peek(|state| state.new_project.clone());

		if input.title.trim().is_empty() {
			self.notice().set(Some("Give the project a title.".into()));

			return;
		}

		self.busy().set(true);
		let store = self.clone();
		spawn(async move {
			match api().create_project(&input).await {
				Ok(project) => {
					store.busy().set(false);
					store.notice().set(None);
					store.new_project().set(NewProjectInput::default());
					store.load_library();
					then(project.id);
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Deletes a project and everything in it.
	pub fn delete_project(&self, id: ProjectId, then: impl FnOnce() + 'static) {
		let store = self.clone();
		spawn(async move {
			match api().delete_project(id).await {
				Ok(()) => {
					store.project().set(None);
					store.load_library();
					then();
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Saves the clip written by hand into the open project.
	pub fn add_clip(&self, project: ProjectId) {
		let draft = self.peek(|state| state.clip.clone());
		let optional = |text: &str| {
			let text = text.trim();
			(!text.is_empty()).then(|| text.to_owned())
		};
		let input = NewClipInput {
			project: Some(project),
			page: PageInput {
				url: draft.url.trim().to_owned(),
				title: draft.title.trim().to_owned(),
				author: optional(&draft.author),
				site: optional(&draft.site),
				published: optional(&draft.published),
			},
			quote: TextQuote {
				exact: draft.quote.trim().to_owned(),
				prefix: None,
				suffix: None,
			},
			note: optional(&draft.note),
			tags: draft.tags.clone(),
		};
		self.busy().set(true);
		let store = self.clone();
		spawn(async move {
			match api().create_clip(&input).await {
				Ok(_) => {
					store.busy().set(false);
					store.notice().set(None);
					store.clip().set(ClipDraft::default());
					store.reload_project(project);
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Saves the note being edited for a clip.
	pub fn save_clip_note(&self, project: ProjectId) {
		let Some((clip, note)) = self.peek(|state| state.editing.clone()) else {
			return;
		};
		let store = self.clone();
		spawn(async move {
			let changes = ClipChanges {
				note: Some(Some(note)),
				tags: None,
			};
			match api().update_clip(clip, &changes).await {
				Ok(_) => {
					store.editing().set(None);
					store.reload_project(project);
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Deletes a clip.
	pub fn delete_clip(&self, project: ProjectId, clip: ClipId, then: impl FnOnce() + 'static) {
		let store = self.clone();
		spawn(async move {
			match api().delete_clip(clip).await {
				Ok(()) => {
					store.reload_project(project);
					then();
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Adds the note in the form to a project.
	pub fn add_note(&self, project: ProjectId) {
		let body = self.peek(|state| state.note.trim().to_owned());

		if body.is_empty() {
			return;
		}

		let store = self.clone();
		spawn(async move {
			match api().create_note(project, &body).await {
				Ok(_) => {
					store.note().update(String::clear);
					store.reload_project(project);
				}
				Err(problem) => store.failed(&problem),
			}
		});
	}

	/// Deletes a note.
	pub fn delete_note(&self, project: ProjectId, note: NoteId) {
		let store = self.clone();
		spawn(async move {
			match api().delete_note(note).await {
				Ok(()) => store.reload_project(project),
				Err(problem) => store.failed(&problem),
			}
		});
	}
}
