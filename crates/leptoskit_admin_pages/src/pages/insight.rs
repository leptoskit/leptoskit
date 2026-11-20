//! A builder: trends, funnels or retention, new or saved. Its actions also
//! add the web overview to a dashboard.
//!
//! Its controls change it in place ([`Swap`](crate::analytics::swap::Swap));
//! Save and Add to dashboard ask through the confirm sheet.

use leptos::prelude::GetValue as _;
use leptos::prelude::SetValue as _;
use leptos::prelude::StoredValue;
use leptoskit::native::FocusRequest;
use leptoskit::prelude::*;
use serde_json::json;

use crate::BASE;
use crate::analytics::insights;
use crate::analytics::insights::Builder;
use crate::analytics::insights::Kind;
use crate::analytics::params;
use crate::components;
use crate::components::Kind as ButtonKind;
use crate::confirm::Answer;
use crate::confirm::Ask;
use crate::confirm::Confirm;
use crate::model::insight::Insight;

/// What Save and Add to dashboard send about the page: the insight as its
/// address asked for it.
#[derive(Clone, Debug, Default)]
struct Shown {
	kind: String,
	/// The saved insight's id.
	insight: Option<String>,
	name: String,
	params: params::Params,
}

impl Shown {
	fn of(insight: &Insight) -> Self {
		Self {
			kind: insight.kind.slug().to_owned(),
			insight: insight.saved.as_ref().map(|saved| saved.id.clone()),
			name: insight.name.clone(),
			params: insight.address.clone(),
		}
	}
}

/// What a builder can do besides change: save it, and add it to a
/// dashboard.
#[derive(Clone, Copy)]
pub(crate) struct Actions {
	/// The insight the page shows now; a swap changes it.
	shown: StoredValue<Shown>,
	save: FocusRequest,
	add: FocusRequest,
	confirm: Confirm,
	notice: RwSignal<String>,
}

impl Actions {
	pub(crate) fn new(confirm: Confirm, notice: RwSignal<String>) -> Self {
		Self {
			shown: StoredValue::new(Shown::default()),
			save: FocusRequest::new(),
			add: FocusRequest::new(),
			confirm,
			notice,
		}
	}

	/// `button`, which adds the web overview the page shows, with `params`,
	/// to a dashboard.
	pub(crate) fn add_web(self, button: Element, params: params::Params) -> Element {
		self.shown.set_value(Shown {
			kind: "web".to_owned(),
			insight: None,
			name: "Web overview".to_owned(),
			params,
		});

		button.focus_on(self.add).on_press(move || self.add())
	}

	/// Asks for a name, then saves the insight and opens it.
	fn save(self) {
		let shown = self.shown.get_value();
		let ask = if shown.insight.is_some() {
			Ask::new(
				"Save changes",
				"Saves what the builder shows now over this insight. Dashboards that hold it \
				 update.",
			)
		} else {
			Ask::new(
				"Save",
				"Saves this insight to the library, where anyone with the admin can open it.",
			)
		}
		.named("Name", shown.name.clone());

		self.confirm
			.ask(ask, Some(self.save), move |answer: Answer| {
				let body = json!({
					"kind": shown.kind,
					"insight": shown.insight,
					"name": answer.text,
					"params": shown.params,
				});
				self.send(
					format!("{BASE}/api/analytics/insights"),
					body.to_string(),
					"The admin couldn't save it. Nothing changed.",
				);
			});
	}

	/// Lists the dashboards, asks which one, then adds the insight to it and
	/// opens it.
	fn add(self) {
		let shown = self.shown.get_value();

		crate::spawn(async move {
			let listed = crate::get_json(&format!("{BASE}/api/analytics/dashboards"), false).await;
			let mut choices: Vec<(String, String)> = listed
				.as_ref()
				.and_then(|(_, data)| data["dashboards"].as_array())
				.into_iter()
				.flatten()
				.map(|dashboard| {
					let cards = dashboard["cards"].as_u64().unwrap_or(0);
					(
						dashboard["id"].as_str().unwrap_or_default().to_owned(),
						format!(
							"{} ({cards} card{})",
							dashboard["name"].as_str().unwrap_or_default(),
							if cards == 1 { "" } else { "s" }
						),
					)
				})
				.filter(|(id, _)| !id.is_empty())
				.collect();
			choices.push((String::new(), "A new dashboard".to_owned()));
			let ask = Ask::new(
				"Add to dashboard",
				format!(
					"Adds “{}” as a card{}.",
					shown.name,
					if shown.insight.is_some() {
						""
					} else {
						", saving it to the library first"
					}
				),
			)
			.choices(choices)
			.named("New dashboard's name", "Product");

			self.confirm
				.ask(ask, Some(self.add), move |answer: Answer| {
					let dashboard = answer.choice.filter(|choice| !choice.is_empty());
					let body = json!({
						"kind": shown.kind,
						"insight": shown.insight,
						"name": shown.name,
						"params": shown.params,
						"dashboard_name": dashboard.is_none().then_some(&answer.text),
						"dashboard": dashboard,
					});
					self.send(
						format!("{BASE}/api/analytics/dashboards"),
						body.to_string(),
						"The admin couldn't add it. Nothing changed.",
					);
				});
		});
	}

	/// Posts `body` to `path` and opens the page the answer names, or says
	/// why not.
	fn send(self, path: String, body: String, failed: &'static str) {
		crate::spawn(async move {
			let answer = crate::post_json(&path, &body).await;
			let href = answer
				.as_ref()
				.filter(|(ok, _)| *ok)
				.and_then(|(_, data)| data["href"].as_str().map(str::to_owned));

			if let Some(href) = href {
				crate::go(&href);
				return;
			}

			self.notice.set(
				answer
					.and_then(|(_, data)| data["message"].as_str().map(str::to_owned))
					.unwrap_or_else(|| failed.to_owned()),
			);
		});
	}
}

/// A builder page.
pub(crate) fn page(insight: &Insight, actions: Option<Actions>) -> Element {
	let params = params::of(&insight.query);
	let here = insight.here();
	let cx = Builder {
		definitions: &insight.definitions,
		notes: &insight.notes,
	};
	let builder = match insight.kind {
		Kind::Trends => insights::trends_builder(&insight.path, &params, &insight.query, &cx),
		Kind::Funnels => insights::funnels_builder(&insight.path, &params, &insight.query, &cx),
		Kind::Retention => insights::retention_builder(&insight.path, &params, &insight.query, &cx),
	};
	let (body, _, aside) = insights::answered(
		insight.kind,
		&insight.query,
		insight.answer.as_ref().map_err(String::as_str),
		&here,
	);
	let chart = components::card(insights::chart_title(&insight.query))
		.aside(aside)
		.workings(insight.kind.slug(), insight.source.clone())
		.test_id("chart-card")
		.child(body)
		.build();

	let mut add = Button("Add to dashboard")
		.class(ButtonKind::Secondary.class_name())
		.test_id("add-to-dashboard");
	let mut save = Button(if insight.saved.is_some() {
		"Save changes"
	} else {
		"Save"
	})
	.class(ButtonKind::Primary.class_name())
	.test_id("save-insight");

	if let Some(actions) = actions {
		actions.shown.set_value(Shown::of(insight));
		add = add.focus_on(actions.add).on_press(move || actions.add());
		save = save.focus_on(actions.save).on_press(move || actions.save());
	}

	View()
		.class("flex-col gap-5")
		.test_id("live-refresh")
		.child(components::header(
			&insight.name,
			&insight.subtitle,
			vec![add, components::raised(save)],
		))
		.child(insights::range_chips(&insight.path, &params))
		.child(builder)
		.child(chart)
}
