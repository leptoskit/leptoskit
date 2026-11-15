//! The admin's connection to the main site's analytics service.
//!
//! Every read and write here is one of the service's app-only operations,
//! called the way the main site's own server calls them: at the service's
//! private URL, signed with its token ([`AppCaller`]). The admin has already
//! checked that the person asking holds an admin role before anything here
//! runs, so the service sees one caller, the app, and the admin decides who
//! that may act for.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;
use std::time::Instant;

use leptoskit_analytics::query::Answer;
use leptoskit_analytics::query::Query;
use leptoskit_analytics::service::query as ops;
use leptoskit_analytics::service::query::DashboardView;
use leptoskit_analytics::service::query::EventDefinitionView;
use leptoskit_analytics::service::query::InsightView;
use leptoskit_analytics::service::read;
use leptoskit_analytics::service::read::EventView;
use leptoskit_analytics::service::read::PersonSummary;
use leptoskit_analytics::service::read::PersonView;
use leptoskit_analytics::service::read::SessionView;
use leptoskit_analytics::service::tables::Tile;
use leptoskit_rpc::FnError;
use leptoskit_rpc::client::AppCaller;
use uuid::Uuid;

use super::live::Hub;
use crate::AnalyticsService;

/// Why a call to the service didn't answer, said for the person reading
/// the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure(pub String);

impl std::fmt::Display for Failure {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

/// What the Learn hub checks lessons against, read in one go and kept for
/// a few seconds so every page's rail can count progress.
#[derive(Clone, Debug, Default)]
pub(crate) struct Facts {
	pub definitions: Vec<EventDefinitionView>,
	pub insights: Vec<InsightView>,
	/// An identified person whose anonymous ids were merged in, by email or
	/// id, when there is one.
	pub merged_person: Option<(Uuid, String)>,
}

/// How long [`Facts`] are reused before they're read again.
const FACTS_FOR: Duration = Duration::from_secs(5);

/// The connection: the service's address, the signed caller, and the live
/// tail the admin shares between every open page.
#[derive(Clone)]
pub(crate) struct Connection {
	service: AnalyticsService,
	caller: AppCaller,
	live: Hub,
	facts: Arc<Mutex<Option<(Instant, Facts)>>>,
}

impl Connection {
	pub(crate) fn new(service: AnalyticsService) -> Self {
		let caller = AppCaller::new(
			leptoskit_analytics::SERVICE,
			service.url.clone(),
			service.token.clone(),
		);
		let live = Hub::new(service.url.clone(), service.token.clone());
		Self {
			service,
			caller,
			live,
			facts: Arc::new(Mutex::new(None)),
		}
	}

	/// The service's origin, for the Workings drawer's HTTP tab.
	pub(crate) fn url(&self) -> &str {
		self.service.url()
	}

	/// The live tail.
	pub(crate) fn live(&self) -> &Hub {
		&self.live
	}

	fn explain(&self, error: &FnError) -> Failure {
		let message = error.message().to_owned();
		Failure(match error {
			FnError::Network(_) => {
				format!(
					"The analytics service at {} didn't answer: {message}. Check that it's \
					 running and that the admin can reach its private URL.",
					self.service.url
				)
			}
			_ if message.contains("only the app") => {
				format!(
					"The analytics service refused the admin's token: {message}. Give the admin \
					 the same LEPTOSKIT_SERVICE_ANALYTICS_TOKEN the main site's server has."
				)
			}
			_ => format!("The analytics service answered with an error: {message}"),
		})
	}

	/// Runs a query: its answer, beside the SQL it ran.
	pub(crate) async fn run(&self, query: Query) -> Result<Answer, Failure> {
		self.caller
			.call(ops::Run { query })
			.await
			.map_err(|error| self.explain(&error))
	}

	/// The event catalogue.
	pub(crate) async fn definitions(&self) -> Result<Vec<EventDefinitionView>, Failure> {
		self.caller
			.call(ops::EventDefinitions {})
			.await
			.map_err(|error| self.explain(&error))
	}

	/// What Learn checks, from the last few seconds when it was read then.
	pub(crate) async fn facts(&self) -> Result<Facts, Failure> {
		if let Some((at, facts)) = self
			.facts
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.as_ref()
			&& at.elapsed() < FACTS_FOR
		{
			return Ok(facts.clone());
		}

		let (definitions, insights, persons) = futures::try_join!(
			self.definitions(),
			self.insights(),
			self.persons(None, None, None, 50, 0),
		)?;
		let mut merged_person = None;

		// A merge shows as an identified person with more than one id; the
		// newest few are enough to find one.
		for person in persons
			.iter()
			.filter(|person| person.is_identified)
			.take(10)
		{
			if let Some(view) = self.person(person.id).await?
				&& view.distinct_ids.len() > 1
			{
				merged_person = Some((person.id, super::format::person_name(&view.person)));
				break;
			}
		}

		let facts = Facts {
			definitions,
			insights,
			merged_person,
		};
		*self.facts.lock().unwrap_or_else(PoisonError::into_inner) =
			Some((Instant::now(), facts.clone()));
		Ok(facts)
	}

	/// Forgets the facts, after a write that changes them.
	pub(crate) fn forget_facts(&self) {
		*self.facts.lock().unwrap_or_else(PoisonError::into_inner) = None;
	}

	pub(crate) async fn insights(&self) -> Result<Vec<InsightView>, Failure> {
		self.caller
			.call(ops::Insights {})
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn insight(&self, id: Uuid) -> Result<Option<InsightView>, Failure> {
		self.caller
			.call(ops::Insight { id })
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn create_insight(
		&self,
		name: String,
		query: Query,
		created_by: String,
	) -> Result<InsightView, Failure> {
		self.forget_facts();
		self.caller
			.call(ops::CreateInsight {
				name,
				description: String::new(),
				query,
				created_by,
			})
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn update_insight(
		&self,
		id: Uuid,
		name: Option<String>,
		query: Option<Query>,
	) -> Result<InsightView, Failure> {
		self.forget_facts();
		self.caller
			.call(ops::UpdateInsight {
				id,
				name,
				description: None,
				query,
			})
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn dashboards(&self) -> Result<Vec<DashboardView>, Failure> {
		self.caller
			.call(ops::Dashboards {})
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn dashboard(&self, id: Uuid) -> Result<Option<DashboardView>, Failure> {
		self.caller
			.call(ops::Dashboard { id })
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn create_dashboard(
		&self,
		name: String,
		tiles: Vec<Tile>,
	) -> Result<DashboardView, Failure> {
		self.forget_facts();
		self.caller
			.call(ops::CreateDashboard { name, tiles })
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn update_dashboard(
		&self,
		id: Uuid,
		tiles: Vec<Tile>,
	) -> Result<DashboardView, Failure> {
		self.forget_facts();
		self.caller
			.call(ops::UpdateDashboard {
				id,
				name: None,
				tiles: Some(tiles),
			})
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn persons(
		&self,
		search: Option<String>,
		property: Option<String>,
		value: Option<String>,
		limit: u32,
		offset: u32,
	) -> Result<Vec<PersonSummary>, Failure> {
		self.caller
			.call(read::Persons {
				search,
				property,
				value,
				limit,
				offset,
			})
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn person(&self, id: Uuid) -> Result<Option<PersonView>, Failure> {
		self.caller
			.call(read::Person { id })
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn person_events(
		&self,
		id: Uuid,
		limit: u32,
	) -> Result<Vec<EventView>, Failure> {
		self.caller
			.call(read::PersonEvents {
				id,
				limit,
				offset: 0,
			})
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn person_sessions(
		&self,
		id: Uuid,
		limit: u32,
	) -> Result<Vec<SessionView>, Failure> {
		self.caller
			.call(read::PersonSessions {
				id,
				limit,
				offset: 0,
			})
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn delete_person(&self, id: Uuid) -> Result<read::Deleted, Failure> {
		self.forget_facts();
		self.caller
			.call(read::DeletePerson { id })
			.await
			.map_err(|error| self.explain(&error))
	}

	pub(crate) async fn export_person(&self, id: Uuid) -> Result<serde_json::Value, Failure> {
		self.caller
			.call(read::ExportPerson { id })
			.await
			.map_err(|error| self.explain(&error))
	}
}
