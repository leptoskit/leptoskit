//! The console's state: made once when the app starts, before the first
//! screen builds, and shared through context.
//!
//! Every query handle is made here, once, in the same order on the server and
//! in the browser, so the server's results stream to the page and hydration
//! picks them up. Each query's key follows the route (see [`Console::new`]
//! for the one that also waits on `whoami`): a screen never makes a handle of
//! its own while it builds.

use std::sync::Arc;

use leptoskit::auth::ErrorResponse;
use leptoskit::auth::IdentityInfo;
use leptoskit::auth::Passkey;
use leptoskit::auth::PasskeyInfo;
use leptoskit::auth::ProviderInfo;
use leptoskit::auth::PublicSession;
use leptoskit::auth::Session;
use leptoskit::auth::StartProvider;
use leptoskit::auth::WalletSettings;
use leptoskit::auth::client::EmailCode;
use leptoskit::auth::client::PasskeyAdded;
use leptoskit::auth::client::PasskeyMode;
use leptoskit::auth::client::SignInStep;
use leptoskit::auth::client::WalletApp;
use leptoskit::db::Id;
use leptoskit::idp::DeviceDecision;
use leptoskit::idp::DeviceRequest;
use leptoskit::leptos::prelude::GetValue as _;
use leptoskit::leptos::prelude::Owner;
use leptoskit::leptos::prelude::StoredValue;
use leptoskit::leptos::prelude::provide_context;
use leptoskit::leptos::prelude::use_context;
use leptoskit::prelude::*;
use leptoskit::query::MutationHandle;
use leptoskit::query::QueryHandle;
use leptoskit::query::QueryObserverOptions;
use leptoskit::query::use_mutation;
use leptoskit::query::use_query_with;
use leptoskit::rpc::FnError;
use leptoskit::rpc::client::Args;
use leptoskit::rpc::client::mutation_scope;
use leptoskit::rpc::client::query_scope;
use leptoskit_cloud::account::CreateOrganization;
use leptoskit_cloud::account::OrganizationSummary;
use leptoskit_cloud::account::Organizations;
use leptoskit_cloud::account::WhoAmI;
use leptoskit_cloud::account::Whoami;
use leptoskit_cloud::projects::ProjectSummary;
use leptoskit_cloud::projects::Projects;

use crate::components::Connection;
use crate::components::Turned;
use crate::data::Billing;
use crate::data::Deployments;
use crate::data::NotAvailableYet;
use crate::routes::Navigation;
use crate::routes::Route;

/// Something a screen waits for: loading, there, or failed with a sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load<T> {
	/// Not here yet.
	Loading,
	/// Here.
	Ready(T),
	/// It failed: what went wrong, in the server's words.
	Failed(String),
}

impl<T: Clone + Send + Sync + 'static> Load<T> {
	/// A query's state.
	pub fn of<E>(handle: &QueryHandle<T, E>, describe: impl Fn(&E) -> String) -> Self
	where
		E: Clone + Send + Sync + 'static,
	{
		match (handle.data(), handle.error()) {
			(Some(data), _) => Self::Ready(data),
			(None, Some(error)) => Self::Failed(describe(&error)),
			(None, None) => Self::Loading,
		}
	}
}

/// An operation's error as the console says it: the server's own message,
/// which the control plane writes in sentence case.
#[must_use]
pub fn rpc_problem(error: &FnError) -> String {
	match error {
		FnError::Network(_) => "rpc: unavailable".to_owned(),
		FnError::Invalid(message)
		| FnError::Server(message)
		| FnError::Decode(message)
		| FnError::Other(message)
		| FnError::Refused { message, .. } => message.clone(),
	}
}

/// A sign-in error as the console says it.
#[must_use]
pub fn auth_problem(error: &ErrorResponse) -> String {
	error.message.clone()
}

/// Where someone is: signed in, out, or not known yet.
#[derive(Clone, Debug, PartialEq)]
pub enum Who {
	/// The session hasn't loaded.
	Checking,
	/// No one is signed in.
	SignedOut,
	/// Someone is.
	SignedIn(PublicSession),
	/// The session couldn't be read.
	Unknown(String),
}

/// The sign-in sheets' state.
#[derive(Clone, Copy)]
pub struct SignInFlow {
	/// The address, as typed.
	pub email: RwSignal<String>,
	/// The emailed code, as typed.
	pub code: RwSignal<String>,
	/// The authenticator app's code, after the emailed code asked for one.
	pub factor_code: RwSignal<String>,
	/// The address the last code went to.
	pub sent_to: RwSignal<Option<String>>,
	/// Whether the account asked for a second step.
	pub second_step: RwSignal<bool>,
	/// What's wrong with what was typed, found before anything is sent.
	pub problem: RwSignal<Option<String>>,
	/// Seconds until another code can be sent.
	pub resend_in: RwSignal<u32>,
	/// The provider being opened, by ID.
	pub opening: RwSignal<Option<String>>,
	/// Emailing a code.
	pub send: MutationHandle<String, (), ErrorResponse>,
	/// Checking the emailed code.
	pub verify: MutationHandle<EmailCode, SignInStep, ErrorResponse>,
	/// Checking the second step's code.
	pub finish: MutationHandle<String, PublicSession, ErrorResponse>,
	/// Going to GitHub or Google.
	pub provider: MutationHandle<StartProvider, (), ErrorResponse>,
	/// Signing in with a passkey.
	pub passkey: MutationHandle<PasskeyMode, PublicSession, ErrorResponse>,
	/// Signing in with a Solana wallet.
	pub wallet: MutationHandle<String, SignInStep, ErrorResponse>,
}

/// The device sheet's state.
#[derive(Clone, Copy)]
pub struct DeviceFlow {
	/// The code, as typed when the link didn't carry one.
	pub typed: RwSignal<String>,
	/// The terminal waiting for the code in the address.
	pub request: QueryHandle<DeviceRequest, ErrorResponse>,
	/// The answer.
	pub decide: MutationHandle<DeviceDecision, (), ErrorResponse>,
}

/// The organisations page's form.
#[derive(Clone, Copy)]
pub struct NewOrganization {
	/// Whether the form is open.
	pub open: RwSignal<bool>,
	/// The name, as typed.
	pub name: RwSignal<String>,
	/// The slug, as typed, or `None` while it follows the name.
	pub slug: RwSignal<Option<String>>,
	/// Making it.
	pub create: MutationHandle<CreateOrganization, OrganizationSummary, FnError>,
}

/// The account page's reads and its one action.
#[derive(Clone, Copy)]
pub struct AccountState {
	/// The providers connected to the account.
	pub identities: QueryHandle<Vec<IdentityInfo>, ErrorResponse>,
	/// Its passkeys.
	pub passkeys: QueryHandle<Vec<PasskeyInfo>, ErrorResponse>,
	/// Where it's signed in.
	pub devices: QueryHandle<Vec<leptoskit::auth::Device>, ErrorResponse>,
	/// Adding a passkey.
	pub add_passkey: MutationHandle<Option<String>, PasskeyAdded, ErrorResponse>,
	/// Removing one.
	pub remove_passkey: MutationHandle<Id<Passkey>, (), ErrorResponse>,
	/// Signing another device out.
	pub revoke_device: MutationHandle<Id<Session>, bool, ErrorResponse>,
	/// Connecting GitHub or Google.
	pub link_provider: MutationHandle<StartProvider, (), ErrorResponse>,
	/// Signing out of this device.
	pub sign_out: MutationHandle<(), (), ErrorResponse>,
}

impl SignInFlow {
	fn new() -> Self {
		Self {
			email: RwSignal::new(String::new()),
			code: RwSignal::new(String::new()),
			factor_code: RwSignal::new(String::new()),
			sent_to: RwSignal::new(None),
			second_step: RwSignal::new(false),
			problem: RwSignal::new(None),
			resend_in: RwSignal::new(0),
			opening: RwSignal::new(None),
			send: use_mutation(leptoskit::auth::client::send_code()),
			verify: use_mutation(leptoskit::auth::client::verify_code()),
			finish: use_mutation(leptoskit::auth::client::complete_sign_in()),
			provider: use_mutation(leptoskit::auth::client::sign_in_with_provider()),
			passkey: use_mutation(leptoskit::auth::client::passkey_sign_in()),
			wallet: use_mutation(leptoskit::auth::client::wallet_sign_in()),
		}
	}
}

impl DeviceFlow {
	/// The flow for the code in the address on `/device`, normalised as the
	/// server reads it.
	fn new(navigation: &Navigation) -> Self {
		let user_code = {
			let navigation = navigation.clone();
			move || {
				match navigation.current() {
					Route::Device(route) => {
						route
							.user_code
							.as_deref()
							.map(normalize_code)
							.filter(|code| !code.is_empty())
					}
					_ => None,
				}
			}
		};

		Self {
			typed: RwSignal::new(String::new()),
			request: use_query_with(
				leptoskit::idp::client::device_request()
					.with_ssr()
					.with_ssr_errors(),
				user_code,
				QueryObserverOptions::default(),
			),
			decide: use_mutation(leptoskit::idp::client::decide_device()),
		}
	}
}

impl NewOrganization {
	fn new() -> Self {
		Self {
			open: RwSignal::new(false),
			name: RwSignal::new(String::new()),
			slug: RwSignal::new(None),
			create: use_mutation(
				mutation_scope::<CreateOrganization>().with_on_success_async(|args| {
					async move {
						args.client.invalidate_scope(&query_scope::<Whoami>()).await;
						args.client
							.invalidate_scope(&query_scope::<Organizations>())
							.await;
					}
				}),
			),
		}
	}
}

impl AccountState {
	/// The account page's reads, asked on `/account` only, in this order.
	fn new(navigation: &Navigation) -> Self {
		let on_account = {
			let navigation = navigation.clone();
			move || matches!(navigation.current(), Route::Account).then_some(())
		};

		Self {
			identities: use_query_with(
				leptoskit::auth::client::identities()
					.with_ssr()
					.with_ssr_errors(),
				on_account.clone(),
				QueryObserverOptions::default(),
			),
			passkeys: use_query_with(
				leptoskit::auth::client::passkeys()
					.with_ssr()
					.with_ssr_errors(),
				on_account.clone(),
				QueryObserverOptions::default(),
			),
			devices: use_query_with(
				leptoskit::auth::client::devices()
					.with_ssr()
					.with_ssr_errors(),
				on_account,
				QueryObserverOptions::default(),
			),
			add_passkey: use_mutation(leptoskit::auth::client::add_passkey()),
			remove_passkey: use_mutation(leptoskit::auth::client::remove_passkey()),
			revoke_device: use_mutation(leptoskit::auth::client::revoke_device()),
			link_provider: use_mutation(leptoskit::auth::client::link_provider()),
			sign_out: use_mutation(leptoskit::auth::client::sign_out_mutation()),
		}
	}
}

/// The operations the control plane doesn't have yet (§20), behind their
/// boundaries.
#[derive(Clone)]
pub struct Services {
	/// `deployments::in_flight`.
	pub deployments: Arc<dyn Deployments>,
	/// `billing::plans` and `usage`.
	pub billing: Arc<dyn Billing>,
}

impl Default for Services {
	fn default() -> Self {
		Self {
			deployments: Arc::new(NotAvailableYet),
			billing: Arc::new(NotAvailableYet),
		}
	}
}

/// Everything the console's screens read and change.
#[derive(Clone, Copy)]
pub struct Console {
	/// Who's signed in.
	pub session: QueryHandle<Option<PublicSession>, ErrorResponse>,
	/// The providers the server set up: GitHub and Google, when configured.
	pub providers: QueryHandle<Vec<ProviderInfo>, ErrorResponse>,
	/// Whether wallets may sign in here.
	pub wallet_settings: QueryHandle<WalletSettings, ErrorResponse>,
	/// The Solana wallets this browser offers, read once the page is live.
	pub wallet_apps: QueryHandle<Vec<WalletApp>, ErrorResponse>,
	/// The person, and their organisations.
	pub whoami: QueryHandle<WhoAmI, FnError>,
	/// The organisations, for `/organizations`.
	pub organizations: QueryHandle<Vec<OrganizationSummary>, FnError>,
	/// The projects of the organisation on screen.
	pub projects: QueryHandle<Vec<ProjectSummary>, FnError>,
	/// The sign-in sheets.
	pub sign_in: SignInFlow,
	/// The device sheet.
	pub device: DeviceFlow,
	/// The organisations page's form.
	pub new_organization: NewOrganization,
	/// The account page.
	pub account: AccountState,
	/// Which cards are turned over.
	pub turned: Turned,
	/// The control whose focus ring shows.
	pub focus: RwSignal<Option<String>>,
	/// The rail's project filter.
	pub filter: RwSignal<String>,
	/// Whether `/` asked for the filter.
	pub find: RwSignal<bool>,
	/// Whether a text field has focus, so the console's keys leave it alone.
	pub typing: RwSignal<bool>,
	/// Whether the page is live: what only the browser knows, such as its
	/// wallets, waits for it, so the server's page and the browser's first
	/// render agree. It starts false where there's a page to agree with (the
	/// server rendering one, the browser hydrating it, until `app::App`
	/// marks it live) and true where there's none, as in a test.
	pub mounted: RwSignal<bool>,
	/// The live connection, for the offline chip.
	pub connection: RwSignal<Connection>,
	/// What a copy button copied last, while that's recent.
	pub copied: RwSignal<Option<String>>,
	services: StoredValue<Services>,
	host: StoredValue<String>,
}

impl Console {
	/// Makes the console's state in the current reactive owner, which holds
	/// the `QueryClient`, with the queries following `navigation`. `host` is
	/// where the console is, such as `cloud.leptoskit.dev`, for sentences
	/// that name an address.
	///
	/// The server's page hands each query's answer to the browser under a
	/// number, given in the order the queries start. The browser starts every
	/// query as it's made, since the answers are already in the page; the
	/// server starts a query whose key waits on another query's answer only
	/// once that answer arrives, and may finish the page before that query
	/// does. So every key here comes from the route alone, and the console
	/// is made after the page's `Suspense` has taken its own number
	/// (`app::App`). The one exception, the projects of the person's own
	/// organisation, waits on `whoami`: it's made last, and asked only once
	/// the page is live, so the server's page and the browser's first render
	/// both go without it. A query a signed-out visit starts fails quietly:
	/// the frame follows the session.
	pub fn new(navigation: &Navigation, services: Services, host: String) -> Self {
		let mounted = RwSignal::new(Owner::current_shared_context().is_none());
		let session = use_query_with(
			leptoskit::auth::client::session()
				.with_ssr()
				.with_ssr_errors(),
			|| (),
			QueryObserverOptions::default(),
		);
		let providers = use_query_with(
			leptoskit::auth::client::providers()
				.with_ssr()
				.with_ssr_errors(),
			|| (),
			QueryObserverOptions::default(),
		);
		let wallet_settings = use_query_with(
			leptoskit::auth::client::wallet_settings()
				.with_ssr()
				.with_ssr_errors(),
			|| (),
			QueryObserverOptions::default(),
		);
		// Only the browser knows its wallets; the server's page has none.
		let wallet_apps = use_query_with(
			leptoskit::auth::client::wallet_apps(),
			move || mounted.get().then_some(()),
			QueryObserverOptions::default(),
		);
		let whoami = use_query_with(
			query_scope::<Whoami>(),
			{
				let navigation = navigation.clone();
				move || shows_an_account(&navigation.current()).then(|| Args::new(Whoami {}))
			},
			QueryObserverOptions::default(),
		);
		let organizations = use_query_with(
			query_scope::<Organizations>(),
			{
				let navigation = navigation.clone();
				move || {
					matches!(navigation.current(), Route::Organizations)
						.then(|| Args::new(Organizations {}))
				}
			},
			QueryObserverOptions::default(),
		);
		let device = DeviceFlow::new(navigation);
		let account = AccountState::new(navigation);
		let sign_in = SignInFlow::new();
		let new_organization = NewOrganization::new();
		// The organisation on screen: the one in the address, or the
		// person's own everywhere else, for the rail and the first-run pages.
		// Away from an organisation's pages this waits on `whoami`, so it's
		// the last query made, and only the live page asks (see above).
		let projects = use_query_with(
			query_scope::<Projects>(),
			{
				let navigation = navigation.clone();
				move || {
					let organization = match navigation.current() {
						Route::Org(route) => route.org,
						Route::Home
						| Route::SignIn(_)
						| Route::SignInEmail(_)
						| Route::NotFound(_) => {
							return None;
						}
						Route::Welcome
						| Route::Device(_)
						| Route::Account
						| Route::Organizations => {
							if !mounted.get() {
								return None;
							}

							whoami.with_data(|me| {
								first_organization(me).map(|org| org.slug.clone())
							})??
						}
					};

					Some(Args::new(Projects { organization }))
				}
			},
			QueryObserverOptions::default(),
		);

		Self {
			session,
			providers,
			wallet_settings,
			wallet_apps,
			whoami,
			organizations,
			projects,
			sign_in,
			device,
			new_organization,
			account,
			turned: Turned::new(),
			focus: RwSignal::new(None),
			filter: RwSignal::new(String::new()),
			find: RwSignal::new(false),
			typing: RwSignal::new(false),
			mounted,
			connection: RwSignal::new(Connection::Fine),
			copied: RwSignal::new(None),
			services: StoredValue::new(services),
			host: StoredValue::new(host),
		}
	}

	/// Shares the state with every screen beneath the current owner.
	#[must_use]
	pub fn provide(self) -> Self {
		provide_context(self);
		self
	}

	/// The state [`provide`](Self::provide) shared.
	///
	/// # Panics
	///
	/// When no owner above this one provided it: the app always does before
	/// its first screen builds.
	#[must_use]
	pub fn get() -> Self {
		use_context::<Self>().expect("the console provides its state before any screen builds")
	}

	/// Who's signed in.
	#[must_use]
	pub fn who(&self) -> Who {
		match (self.session.data(), self.session.error()) {
			(Some(Some(session)), _) => Who::SignedIn(session),
			(Some(None), _) => Who::SignedOut,
			(None, Some(error)) => Who::Unknown(auth_problem(&error)),
			(None, None) => Who::Checking,
		}
	}

	/// The person and their organisations.
	#[must_use]
	pub fn me(&self) -> Load<WhoAmI> {
		Load::of(&self.whoami, rpc_problem)
	}

	/// The operations behind boundaries.
	#[must_use]
	pub fn services(&self) -> Services {
		self.services.get_value()
	}

	/// Where the console is, such as `cloud.leptoskit.dev`.
	#[must_use]
	pub fn host(&self) -> String {
		self.host.get_value()
	}
}

/// Whether `route` shows someone's account, so the console asks `whoami`:
/// every page but the two sign-in sheets.
#[must_use]
pub const fn shows_an_account(route: &Route) -> bool {
	!matches!(route, Route::SignIn(_) | Route::SignInEmail(_))
}

/// The organisation `/` and the first-run pages use: the personal one, or
/// else the first.
#[must_use]
pub fn first_organization(me: &WhoAmI) -> Option<&OrganizationSummary> {
	me.organizations
		.iter()
		.find(|organization| organization.personal)
		.or_else(|| me.organizations.first())
}

/// Whether the account looks new: only its personal organisation, with no
/// projects. The control plane doesn't say when an account was made, so the
/// first-run pages read it from what the account has.
#[must_use]
pub fn looks_new(me: &WhoAmI, projects: Option<&[ProjectSummary]>) -> bool {
	me.organizations.len() == 1
		&& me
			.organizations
			.iter()
			.all(|organization| organization.personal)
		&& projects.is_some_and(<[ProjectSummary]>::is_empty)
}

/// A typed device code as the server reads it: letters, digits and the dash,
/// in capitals, with the dash put back when it was left out.
#[must_use]
pub fn normalize_code(code: &str) -> String {
	let tidy: String = code
		.chars()
		.filter(char::is_ascii_alphanumeric)
		.map(|c| c.to_ascii_uppercase())
		.collect();

	if tidy.len() == 8 {
		format!("{}-{}", &tidy[..4], &tidy[4..])
	} else {
		tidy
	}
}

/// A device code spelled out for a screen reader: "W D J B, M J H T".
#[must_use]
pub fn spell_code(code: &str) -> String {
	code.split('-')
		.map(|part| part.chars().map(String::from).collect::<Vec<_>>().join(" "))
		.collect::<Vec<_>>()
		.join(", ")
}

/// `next`, when it's a path on this site: never another site, so a link
/// can't send anyone elsewhere after they sign in.
#[must_use]
pub fn local_path(next: Option<&str>) -> Option<String> {
	let next = next?;
	let local = next.starts_with('/') && !next.starts_with("//") && !next.contains('\\');
	local.then(|| next.to_owned())
}

/// The route a local path names, if any.
#[must_use]
pub fn route_of(path: &str) -> Option<Route> {
	use leptoskit_router::Routable as _;

	Route::parse(path).ok()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn codes_are_read_in_any_case_with_or_without_the_dash() {
		assert_eq!(normalize_code("wdjb-mjht"), "WDJB-MJHT");
		assert_eq!(normalize_code("wdjbmjht"), "WDJB-MJHT");
		assert_eq!(normalize_code(" WDJB MJHT "), "WDJB-MJHT");
		assert_eq!(normalize_code("abc"), "ABC");
		assert_eq!(normalize_code(""), "");
	}

	#[test]
	fn codes_are_spelled_out_for_screen_readers() {
		assert_eq!(spell_code("WDJB-MJHT"), "W D J B, M J H T");
	}

	#[test]
	fn only_local_paths_are_followed_after_sign_in() {
		assert_eq!(
			local_path(Some("/device?user_code=WDJB-MJHT")).as_deref(),
			Some("/device?user_code=WDJB-MJHT")
		);
		assert_eq!(local_path(Some("//evil.example")), None);
		assert_eq!(local_path(Some("https://evil.example")), None);
		assert_eq!(local_path(Some("/\\evil.example")), None);
		assert_eq!(local_path(None), None);
	}
}
