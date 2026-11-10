//! The studio's pages: leptos views rendered to HTML on the server, with
//! links and forms for every interaction, and no script.
//!
//! The studio runs inside `cargo test` of the app's db crate, which has no
//! wasm toolchain or bundler in its path, so the pages aren't hydrated; a
//! live page reloads itself instead.

use leptos::prelude::*;
use leptoskit_db_schema::Schema;

use super::Change;
use super::Viewer;
use super::ops::Browse;
use super::ops::Cell;
use super::ops::Edit;
use super::ops::Insert;
use super::ops::Kind;
use super::ops::Op;
use super::ops::Rows;
use super::ops::Rules;
use crate::AuditEntry;
use crate::Persona;

/// What a page shows below the header.
pub(crate) enum Body {
	/// Why the page can't show what it was asked for.
	Problem(String),
	Tables(Vec<(String, Kind, Result<u64, String>)>),
	Rows {
		table: String,
		kind: Kind,
		browse: Browse,
		rows: Rows,
	},
	EditForm {
		table: String,
		key: Vec<String>,
		column: String,
		current: String,
	},
	ConfirmEdit {
		table: String,
		change: Edit,
	},
	InsertForm {
		table: String,
		/// The columns an insert provides.
		columns: Vec<leptoskit_db_schema::Column>,
	},
	ConfirmInsert {
		table: String,
		row: Insert,
	},
	ConfirmDelete {
		table: String,
		key: Vec<String>,
	},
	Done {
		table: String,
		message: String,
	},
	Schema(Schema),
	Migrations(Vec<MigrationRow>),
	Policies(Vec<(String, Result<Rules, String>)>),
	Audit {
		entries: Vec<AuditEntry>,
		page: u64,
	},
	Live(Vec<Change>),
}

/// One migration, and whether the database has it.
pub(crate) struct MigrationRow {
	pub(crate) name: String,
	pub(crate) applied: bool,
	/// Its SQL, when the page shows it.
	pub(crate) sql: Option<String>,
}

/// A whole page.
pub(crate) struct Frame<'a> {
	pub(crate) token: &'a str,
	pub(crate) viewer: &'a Viewer,
	/// Whether `private` columns show their values: `--reveal`.
	pub(crate) reveal: bool,
	pub(crate) title: &'a str,
	pub(crate) personas: Vec<&'static Persona>,
	pub(crate) body: Body,
}

const STYLE: &str = "
body { font: 14px/1.45 system-ui, sans-serif; margin: 0; color: #1d1d1f; background: #fafafa; }
header { background: #1d1d1f; color: #fff; padding: 10px 20px; display: flex; gap: 16px; \
                     align-items: center; flex-wrap: wrap; }
header a { color: #fff; text-decoration: none; }
header strong { margin-right: 12px; }
main { padding: 16px 20px; }
form.viewer { display: flex; gap: 6px; align-items: center; margin-left: auto; }
.viewing { background: #fff3cd; padding: 6px 20px; border-bottom: 1px solid #e0d39a; }
.editing { background: #f8d7da; }
table { border-collapse: collapse; background: #fff; }
td, th { border: 1px solid #ddd; padding: 4px 8px; text-align: left; vertical-align: top; \
                     max-width: 360px; overflow-wrap: anywhere; }
th { background: #f0f0f0; }
.null, .secret { color: #888; font-style: italic; }
.problem { color: #9b1c1c; background: #fdecea; padding: 10px; border-radius: 4px; }
.done { color: #1c5e20; background: #e7f4e8; padding: 10px; border-radius: 4px; }
pre { background: #fff; border: 1px solid #ddd; padding: 8px; overflow: auto; }
nav.pages { margin: 10px 0; display: flex; gap: 12px; }
";

/// Text for a URL's query: everything but unreserved characters encoded.
fn encode(text: &str) -> String {
	text.bytes()
		.map(|byte| {
			if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
				char::from(byte).to_string()
			} else {
				format!("%{byte:02X}")
			}
		})
		.collect()
}

/// Builds the links and form fields that carry the token and viewer.
#[derive(Clone)]
struct Links {
	token: String,
	viewer: Viewer,
}

impl Links {
	/// `path`, with the token, the viewer, and `extra`.
	fn href(&self, path: &str, extra: &[(&str, String)]) -> String {
		self.with_viewer(path, &self.viewer, extra)
	}

	fn with_viewer(&self, path: &str, viewer: &Viewer, extra: &[(&str, String)]) -> String {
		let mut query = format!("token={}", self.token);

		for (name, value) in viewer
			.params()
			.iter()
			.map(|(name, value)| (*name, value.as_str()))
			.chain(extra.iter().map(|(name, value)| (*name, value.as_str())))
		{
			query.push('&');
			query.push_str(name);
			query.push('=');
			query.push_str(&encode(value));
		}

		format!("{path}?{query}")
	}

	/// Hidden fields carrying the viewer, and for a `GET` form the token,
	/// which a `GET` form drops from its action.
	fn hidden(&self, with_token: bool) -> impl IntoView + use<> {
		let token = with_token.then(|| {
			view! { <input type="hidden" name="token" value=self.token.clone() /> }
		});
		let fields = self
			.viewer
			.params()
			.into_iter()
			.map(|(name, value)| view! { <input type="hidden" name=name value=value /> })
			.collect_view();
		view! {
			{token}
			{fields}
		}
	}
}

/// Renders a page to a complete HTML document.
pub(crate) fn render(frame: &Frame<'_>) -> String {
	let links = Links {
		token: frame.token.to_string(),
		viewer: frame.viewer.clone(),
	};
	let title = format!("{} · {}", frame.title, super::TITLE);
	let refresh = matches!(frame.body, Body::Live(_)).then(|| {
		view! { <meta http-equiv="refresh" content="2" /> }
	});
	let header = header(&links, frame.viewer, frame.reveal, &frame.personas);
	let body = body(&links, frame);
	let page = view! {
		<html lang="en">
			<head>
				<meta charset="utf-8" />
				<title>{title}</title>
				{refresh}
				<style inner_html=STYLE />
			</head>
			<body>
				{header}
				<main>
					<h1>{frame.title.to_string()}</h1>
					{body}
				</main>
			</body>
		</html>
	};

	format!("<!DOCTYPE html>{}", page.to_html().replace("<!>", ""))
}

/// The navigation, the viewer form, and the banner saying who reads.
fn header(
	links: &Links,
	viewer: &Viewer,
	reveal: bool,
	personas: &[&'static Persona],
) -> impl IntoView + use<> {
	let nav = [
		("/", "Tables"),
		("/schema", "Schema"),
		("/migrations", "Migrations"),
		("/policies", "Policies"),
		("/audit", "Audit log"),
		("/live", "Live"),
	]
	.into_iter()
	.map(|(path, label)| view! { <a href=links.href(path, &[])>{label}</a> })
	.collect_view();
	let kinds: Vec<(String, String)> = ["anonymous", "system"]
		.into_iter()
		.map(|kind| (kind.to_string(), kind.to_string()))
		.chain(personas.iter().map(|persona| {
			(
				persona.name.to_string(),
				format!("{}: {}", persona.name, persona.describe),
			)
		}))
		.collect();
	let options = kinds
		.into_iter()
		.map(|(kind, label)| {
			let selected = kind == viewer.kind;
			view! { <option value=kind selected=selected>{label}</option> }
		})
		.collect_view();
	let edit_toggle = {
		let mut toggled = viewer.clone();
		toggled.edit = !viewer.edit;
		let label = if viewer.edit {
			"Leave edit mode"
		} else {
			"Edit mode"
		};
		view! { <a href=links.with_viewer("/", &toggled, &[])>{label}</a> }
	};
	let banner_class = if viewer.edit {
		"viewing editing"
	} else {
		"viewing"
	};
	let mut banner = if viewer.edit {
		format!(
			"Reading as {}. Edit mode is on: each change asks you to confirm it, then runs as \
			 this viewer.",
			viewer.describe()
		)
	} else {
		format!("Reading as {}.", viewer.describe())
	};
	banner.push_str(if reveal {
		" Private values are revealed: the studio was started with --reveal."
	} else {
		" Private values show as ‹private›; start the studio with --reveal to see them."
	});

	view! {
		<header>
			<strong>{super::TITLE}</strong>
			{nav}
			{edit_toggle}
			<form class="viewer" method="get" action="/">
				<input type="hidden" name="token" value=links.token.clone() />
				{viewer
					.edit
					.then(|| view! { <input type="hidden" name="edit" value="1" /> })}
				<label>"View as " <select name="as">{options}</select></label>
				<input name="who" placeholder="who (for a persona)" value=viewer.who.clone() />
				<input name="reason" placeholder="why (for system)" value=viewer.reason.clone() />
				<button>"Switch"</button>
			</form>
		</header>
		<div class=banner_class>{banner}</div>
	}
}

fn body(links: &Links, frame: &Frame<'_>) -> AnyView {
	match &frame.body {
		Body::Problem(problem) => view! { <p class="problem">{problem.clone()}</p> }.into_any(),
		Body::Tables(tables) => tables_view(links, tables).into_any(),
		Body::Rows {
			table,
			kind,
			browse,
			rows,
		} => rows_view(links, frame.viewer, table, *kind, browse, rows).into_any(),
		Body::EditForm {
			table,
			key,
			column,
			current,
		} => edit_form_view(links, table, key, column, current).into_any(),
		Body::ConfirmEdit { table, change } => confirm_edit_view(links, table, change).into_any(),
		Body::InsertForm { table, columns } => insert_form_view(links, table, columns).into_any(),
		Body::ConfirmInsert { table, row } => confirm_insert_view(links, table, row).into_any(),
		Body::ConfirmDelete { table, key } => confirm_delete_view(links, table, key).into_any(),
		Body::Done { table, message } => {
			view! {
				<p class="done">{message.clone()}</p>
				<p>
					<a href=links.href(&format!("/table/{table}"), &[])>"Back to the table"</a>
				</p>
			}
			.into_any()
		}
		Body::Schema(schema) => schema_view(schema).into_any(),
		Body::Migrations(migrations) => migrations_view(links, migrations).into_any(),
		Body::Policies(tables) => policies_view(tables).into_any(),
		Body::Audit { entries, page } => audit_view(links, entries, *page).into_any(),
		Body::Live(changes) => live_view(changes).into_any(),
	}
}

fn tables_view(
	links: &Links,
	tables: &[(String, Kind, Result<u64, String>)],
) -> impl IntoView + use<> {
	let rows = |views: bool| {
		tables
			.iter()
			.filter(|(_, kind, _)| (*kind != Kind::Table) == views)
			.map(|(name, kind, count)| {
				let count = match count {
					Ok(count) => count.to_string(),
					Err(error) => error.clone(),
				};
				let kind = views.then(|| view! { <td>{kind.label()}</td> });
				view! {
					<tr>
						<td>
							<a href=links.href(&format!("/table/{name}"), &[])>{name.clone()}</a>
						</td>
						{kind}
						<td>{count}</td>
					</tr>
				}
			})
			.collect_view()
	};
	let views = tables
		.iter()
		.any(|(_, kind, _)| *kind != Kind::Table)
		.then(|| {
			view! {
				<h2>"Views"</h2>
				<p>"Read-only: each reads through its own policies."</p>
				<table>
					<tr>
						<th>"View"</th>
						<th>"Kind"</th>
						<th>"Rows"</th>
					</tr>
					{rows(true)}
				</table>
			}
		});
	view! {
		<p>"Each count is what the viewer may read: the table's policies apply."</p>
		<table>
			<tr>
				<th>"Table"</th>
				<th>"Rows"</th>
			</tr>
			{rows(false)}
		</table>
		{views}
	}
}

fn cell_view(cell: &Cell) -> impl IntoView + use<> {
	let class = match cell {
		Cell::Null => "null",
		Cell::Secret | Cell::Private => "secret",
		Cell::Text(_) => "",
	};
	view! { <span class=class>{cell.text().to_string()}</span> }
}

fn rows_view(
	links: &Links,
	viewer: &Viewer,
	table: &str,
	kind: Kind,
	browse: &Browse,
	rows: &Rows,
) -> impl IntoView + use<> {
	let path = format!("/table/{table}");
	// Nothing writes to a view, so it offers no changes, even in edit mode.
	let editing = viewer.edit && kind == Kind::Table;
	let filter = browse.filter.clone();
	let mut state: Vec<(&str, String)> = Vec::new();

	if let Some(filter) = &filter {
		state.push(("column", filter.column.clone()));
		state.push(("op", filter.op.name().to_string()));
		state.push(("value", filter.value.clone()));
	}

	if browse.with_deleted {
		state.push(("deleted", "1".into()));
	}

	let page = browse.offset / super::PAGE;
	let columns = rows.columns.iter().enumerate().map(|(position, column)| {
		// Sorting by a hidden column would tell its values apart.
		if rows.hidden.contains(&position) {
			return view! { <th>{(*column).to_string()}</th> }.into_any();
		}

		let descending = browse
			.order
			.as_ref()
			.is_some_and(|(by, descending)| by == column && !descending);
		let mut params = state.clone();
		params.push(("order", (*column).to_string()));

		if descending {
			params.push(("desc", "1".into()));
		}

		view! {
			<th>
				<a href=links.href(&path, &params)>{(*column).to_string()}</a>
			</th>
		}
		.into_any()
	});
	let columns = columns.collect_view();
	let edit_header = editing.then(|| view! { <th>"Row"</th> });
	let body = rows
		.cells
		.iter()
		.map(|row| {
			let key: Vec<String> = rows
				.key
				.iter()
				.map(|position| row[*position].text().to_string())
				.collect();
			let key = serde_json::to_string(&key).unwrap_or_default();
			// A row is named by its key, so one whose key the page hides
			// offers no changes.
			let editing = editing && rows.key.iter().all(|position| !row[*position].hidden());
			let cells = row
				.iter()
				.zip(&rows.columns)
				.enumerate()
				.map(|(position, (cell, column))| {
					let editable = editing && !rows.key.contains(&position) && !cell.hidden();
					let edit = editable.then(|| {
						let href = links.href(
							&format!("{path}/edit"),
							&[
								("key", key.clone()),
								("column", (*column).to_string()),
								("current", cell.text().to_string()),
							],
						);
						view! {
							" "
							<a href=href>"edit"</a>
						}
					});
					view! { <td>{cell_view(cell)}{edit}</td> }
				})
				.collect_view();
			let delete = editing.then(|| {
				let href = links.href(&format!("{path}/delete"), &[("key", key.clone())]);
				view! {
					<td>
						<a href=href>"delete"</a>
					</td>
				}
			});
			view! { <tr>{cells}{delete}</tr> }
		})
		.collect_view();
	let shown_from = browse.offset + 1;
	let shown_to = browse.offset + rows.cells.len() as u64;
	let previous = (page > 0).then(|| {
		let mut params = state.clone();
		params.push(("page", (page - 1).to_string()));
		view! { <a href=links.href(&path, &params)>"← Previous"</a> }
	});
	let next = (shown_to < rows.total).then(|| {
		let mut params = state.clone();
		params.push(("page", (page + 1).to_string()));
		view! { <a href=links.href(&path, &params)>"Next →"</a> }
	});
	let deleted_toggle = {
		let mut params: Vec<(&str, String)> = state
			.iter()
			.filter(|(name, _)| *name != "deleted")
			.cloned()
			.collect();
		let label = if browse.with_deleted {
			"Hide soft-deleted rows"
		} else {
			params.push(("deleted", "1".into()));
			"Show soft-deleted rows"
		};
		view! { <a href=links.href(&path, &params)>{label}</a> }
	};
	let insert = editing.then(|| {
		view! { <a href=links.href(&format!("{path}/insert"), &[])>"Insert a row"</a> }
	});
	let shown: Vec<&'static str> = rows
		.columns
		.iter()
		.enumerate()
		.filter(|(position, _)| !rows.hidden.contains(position))
		.map(|(_, column)| *column)
		.collect();

	let read_only = (kind != Kind::Table).then(|| {
		let refreshed = if kind == Kind::Materialized {
			" It holds the rows of its last refresh."
		} else {
			""
		};
		view! {
			<p>
				{format!(
					"A {}: read-only, through its own policies.{refreshed}",
					kind.label()
				)}
			</p>
		}
	});

	view! {
		{read_only}
		{filter_view(links, &path, &shown, browse)}
		<nav class="pages">
			<span>{format!("Rows {shown_from}–{shown_to} of {}", rows.total)}</span>
			{previous}
			{next}
			{deleted_toggle}
			{insert}
		</nav>
		<table>
			<tr>{columns}{edit_header}</tr>
			{body}
		</table>
	}
}

/// The form that filters a table page by one of `columns`.
fn filter_view(
	links: &Links,
	path: &str,
	columns: &[&'static str],
	browse: &Browse,
) -> impl IntoView + use<> {
	let filter = browse.filter.as_ref();
	let column_options = columns
		.iter()
		.map(|column| {
			let selected = filter.is_some_and(|filter| filter.column == *column);
			view! { <option value=*column selected=selected>{*column}</option> }
		})
		.collect_view();
	let op_options = Op::ALL
		.into_iter()
		.map(|op| {
			let selected = filter.is_some_and(|filter| filter.op == op);
			view! { <option value=op.name() selected=selected>{op.label()}</option> }
		})
		.collect_view();
	let value = filter
		.map(|filter| filter.value.clone())
		.unwrap_or_default();
	view! {
		<form method="get" action=path.to_string()>
			{links.hidden(true)} "Filter: " <select name="column">{column_options}</select>
			<select name="op">{op_options}</select> <input name="value" value=value />
			{browse
				.with_deleted
				.then(|| view! { <input type="hidden" name="deleted" value="1" /> })}
			<button>"Apply"</button> " " <a href=links.href(path, &[])>"Clear"</a>
		</form>
	}
}

/// The fields a `POST` form sends besides its own.
fn post_fields(links: &Links) -> impl IntoView + use<> {
	links.hidden(false)
}

fn edit_form_view(
	links: &Links,
	table: &str,
	key: &[String],
	column: &str,
	current: &str,
) -> impl IntoView + use<> {
	let action = links.href(&format!("/table/{table}/edit"), &[]);
	let key = serde_json::to_string(key).unwrap_or_default();
	view! {
		<p>{format!("Change `{column}` of the row keyed {key}.")}</p>
		<form method="post" action=action>
			{post_fields(links)} <input type="hidden" name="key" value=key />
			<input type="hidden" name="column" value=column.to_string() />
			<input type="hidden" name="current" value=current.to_string() />
			<p>
				<label>
					"New value " <input name="value" value=current.to_string() size="60" />
				</label>
			</p>
			<p>
				<label>
					<input type="checkbox" name="null" value="1" />
					" Set it to NULL"
				</label>
			</p>
			<button>"Review the change"</button>
		</form>
	}
}

fn confirm_edit_view(links: &Links, table: &str, change: &Edit) -> impl IntoView + use<> {
	let action = links.href(&format!("/table/{table}/edit"), &[]);
	let key = serde_json::to_string(&change.key).unwrap_or_default();
	let value = change.value.clone();
	let shown = value.clone().unwrap_or_else(|| "NULL".into());
	view! {
		<p class="problem">
			{format!(
				"Set `{}` of the `{table}` row keyed {} to {shown}, as {}? It runs through the \
				 table's save, with its hooks, rules, policies, and audit log.",
				change.column,
				change.key.join(", "),
				links.viewer.describe()
			)}
		</p>
		<form method="post" action=action>
			{post_fields(links)} <input type="hidden" name="key" value=key />
			<input type="hidden" name="column" value=change.column.clone() />
			{match value {
				Some(value) => {
					view! { <input type="hidden" name="value" value=value /> }.into_any()
				}
				None => view! { <input type="hidden" name="null" value="1" /> }.into_any(),
			}}
			<input type="hidden" name="confirmed" value="yes" />
			<button>"Confirm the change"</button> " "
			<a href=links.href(&format!("/table/{table}"), &[])>"Cancel"</a>
		</form>
	}
}

fn insert_form_view(
	links: &Links,
	table: &str,
	columns: &[leptoskit_db_schema::Column],
) -> impl IntoView + use<> {
	let action = links.href(&format!("/table/{table}/insert"), &[]);
	let fields = columns
		.iter()
		.map(|column| {
			let null = column.nullable.then(|| {
				view! {
					<label>
						<input type="checkbox" name=format!("null.{}", column.name) value="1" />
						" NULL"
					</label>
				}
			});
			view! {
				<tr>
					<td>
						<label for=format!("column.{}", column.name)>{column.name.clone()}</label>
					</td>
					<td>{format!("{:?}", column.ty)}</td>
					<td>
						<input
							id=format!("column.{}", column.name)
							name=format!("column.{}", column.name)
							size="60"
						/>
						" "
						{null}
					</td>
				</tr>
			}
		})
		.collect_view();
	view! {
		<p>
			{format!(
				"A new `{table}` row. The key, timestamps, version, and computed columns are \
				 filled in as the table's insert fills them."
			)}
		</p>
		<form method="post" action=action>
			{post_fields(links)}
			<table>
				<tr>
					<th>"Column"</th>
					<th>"Type"</th>
					<th>"Value"</th>
				</tr>
				{fields}
			</table>
			<p>
				<button>"Review the insert"</button>
			</p>
		</form>
	}
}

fn confirm_insert_view(links: &Links, table: &str, row: &Insert) -> impl IntoView + use<> {
	let action = links.href(&format!("/table/{table}/insert"), &[]);
	let shown = row
		.values
		.iter()
		.map(|(column, value)| {
			let cell = match value {
				Some(value) => Cell::Text(value.clone()),
				None => Cell::Null,
			};
			view! {
				<tr>
					<td>{column.clone()}</td>
					<td>{cell_view(&cell)}</td>
				</tr>
			}
		})
		.collect_view();
	let fields = row
		.values
		.iter()
		.map(|(column, value)| {
			match value {
				Some(value) => {
					view! {
						<input type="hidden" name=format!("column.{column}") value=value.clone() />
					}
					.into_any()
				}
				None => {
					view! { <input type="hidden" name=format!("null.{column}") value="1" /> }
						.into_any()
				}
			}
		})
		.collect_view();
	view! {
		<p class="problem">
			{format!(
				"Insert this `{table}` row, as {}? It runs through the table's insert, with its \
				 hooks, rules, policies, and audit log.",
				links.viewer.describe()
			)}
		</p>
		<table>
			<tr>
				<th>"Column"</th>
				<th>"Value"</th>
			</tr>
			{shown}
		</table>
		<form method="post" action=action>
			{post_fields(links)} {fields} <input type="hidden" name="confirmed" value="yes" />
			<p>
				<button>"Confirm the insert"</button>
				" "
				<a href=links.href(&format!("/table/{table}"), &[])>"Cancel"</a>
			</p>
		</form>
	}
}

fn confirm_delete_view(links: &Links, table: &str, key: &[String]) -> impl IntoView + use<> {
	let action = links.href(&format!("/table/{table}/delete"), &[]);
	let json = serde_json::to_string(key).unwrap_or_default();
	view! {
		<p class="problem">
			{format!(
				"Delete the `{table}` row keyed {}, as {}? A table with a soft-delete column \
				 soft-deletes it; otherwise it's gone for good.",
				key.join(", "),
				links.viewer.describe()
			)}
		</p>
		<form method="post" action=action>
			{post_fields(links)} <input type="hidden" name="key" value=json />
			<input type="hidden" name="confirmed" value="yes" />
			<button>"Confirm the delete"</button> " "
			<a href=links.href(&format!("/table/{table}"), &[])>"Cancel"</a>
		</form>
	}
}

fn schema_view(schema: &Schema) -> impl IntoView + use<> {
	let tables = schema
		.tables
		.iter()
		.map(|table| {
			let columns = table
				.columns
				.iter()
				.map(|column| {
					let key = table.primary_key.contains(&column.name);
					view! {
						<tr>
							<td>{column.name.clone()}{key.then_some(" (key)")}</td>
							<td>{format!("{:?}", column.ty)}</td>
							<td>{if column.nullable { "null" } else { "not null" }}</td>
							<td>{column.comment.clone().unwrap_or_default()}</td>
						</tr>
					}
				})
				.collect_view();
			let links = table
				.foreign_keys
				.iter()
				.map(|key| {
					view! {
						<li>
							{format!(
								"{} → {} ({}), on delete {:?}",
								key.columns.join(", "),
								key.references,
								key.referenced_columns.join(", "),
								key.on_delete
							)}
						</li>
					}
				})
				.collect_view();
			let indexes = table
				.indexes
				.iter()
				.map(|index| {
					view! {
						<li>
							{format!(
								"{}{} ({})",
								if index.unique { "unique " } else { "" },
								index.name,
								index.columns.join(", ")
							)}
						</li>
					}
				})
				.collect_view();
			view! {
				<h2>{table.name.to_string()}</h2>
				<p>{table.comment.clone().unwrap_or_default()}</p>
				<table>
					<tr>
						<th>"Column"</th>
						<th>"Type"</th>
						<th>"Null"</th>
						<th>"About"</th>
					</tr>
					{columns}
				</table>
				<ul>{links}{indexes}</ul>
			}
		})
		.collect_view();
	let views = schema
		.views
		.iter()
		.map(|view| {
			let columns = view
				.columns
				.iter()
				.map(|column| {
					view! {
						<tr>
							<td>{column.name.clone()}</td>
							<td>{format!("{:?}", column.ty)}</td>
							<td>{if column.nullable { "null" } else { "not null" }}</td>
						</tr>
					}
				})
				.collect_view();
			let sql = match schema.dialect {
				leptoskit_db_schema::Dialect::Postgres => view.sql.postgres.trim().to_string(),
				leptoskit_db_schema::Dialect::Sqlite => view.sql.sqlite.trim().to_string(),
			};
			view! {
				<h2>
					{view.name.to_string()}
					{if view.materialized {
						" (materialized view)"
					} else {
						" (view)"
					}}
				</h2>
				<p>{view.comment.clone().unwrap_or_default()}</p>
				<table>
					<tr>
						<th>"Column"</th>
						<th>"Type"</th>
						<th>"Null"</th>
					</tr>
					{columns}
				</table>
				<pre>{sql}</pre>
			}
		})
		.collect_view();
	view! {
		<p>"The tables this build of the app declares, as the next migration would see them."</p>
		{tables}
		{views}
	}
}

fn migrations_view(links: &Links, migrations: &[MigrationRow]) -> impl IntoView + use<> {
	let rows = migrations
		.iter()
		.map(|migration| {
			let sql = migration.sql.clone().map(|sql| {
				view! {
					<tr>
						<td colspan="2">
							<pre>{sql}</pre>
						</td>
					</tr>
				}
			});
			view! {
				<tr>
					<td>
						<a href=links.href("/migrations", &[("show", migration.name.clone())])>
							{migration.name.clone()}
						</a>
					</td>
					<td>
						{if migration.applied {
							"applied"
						} else {
							"pending"
						}}
					</td>
				</tr>
				{sql}
			}
		})
		.collect_view();
	view! {
		<table>
			<tr>
				<th>"Migration"</th>
				<th>"This database"</th>
			</tr>
			{rows}
		</table>
	}
}

fn policies_view(tables: &[(String, Result<Rules, String>)]) -> impl IntoView + use<> {
	let tables = tables
		.iter()
		.map(|(name, rules)| {
			let rules = match rules {
				Err(error) => view! { <p class="problem">{error.clone()}</p> }.into_any(),
				Ok(rules) => {
					rules
						.iter()
						.map(|(action, statement)| {
							let shown = match statement {
								None => "every row: no policy restricts it".to_string(),
								Some(statement) if statement.args.is_empty() => {
									statement.sql.clone()
								}
								Some(statement) => {
									format!("{}\n-- with {:?}", statement.sql, statement.args)
								}
							};
							view! {
								<h3>{format!("{action:?}")}</h3>
								<pre>{shown}</pre>
							}
						})
						.collect_view()
						.into_any()
				}
			};
			view! {
				<h2>{name.clone()}</h2>
				{rules}
			}
		})
		.collect_view();
	view! {
		<p>"What this viewer may do to each table, as the SQL the database layer adds for them."</p>
		{tables}
	}
}

fn audit_view(links: &Links, entries: &[AuditEntry], page: u64) -> impl IntoView + use<> {
	let rows = entries
		.iter()
		.map(|entry| {
			let json = |value: &Option<crate::Json<serde_json::Value>>| {
				value
					.as_ref()
					.map(|value| value.0.to_string())
					.unwrap_or_default()
			};
			view! {
				<tr>
					<td>{entry.at.to_rfc3339()}</td>
					<td>{entry.table_name.clone()}</td>
					<td>{entry.row_key.clone()}</td>
					<td>{entry.op.to_string()}</td>
					<td>
						{entry
							.actor
							.map(|actor| actor.to_string())
							.unwrap_or_default()}
					</td>
					<td>{entry.reason.clone().unwrap_or_default()}</td>
					<td>
						<code>{json(&entry.before)}</code>
					</td>
					<td>
						<code>{json(&entry.after)}</code>
					</td>
				</tr>
			}
		})
		.collect_view();
	let previous = (page > 0).then(|| {
		view! { <a href=links.href("/audit", &[("page", (page - 1).to_string())])>"← Newer"</a> }
	});
	let next = (entries.len() as u64 == super::PAGE).then(|| {
		view! { <a href=links.href("/audit", &[("page", (page + 1).to_string())])>"Older →"</a> }
	});
	view! {
		<p>"Newest first, as this viewer may read them."</p>
		<nav class="pages">{previous}{next}</nav>
		<table>
			<tr>
				<th>"When"</th>
				<th>"Table"</th>
				<th>"Row"</th>
				<th>"Change"</th>
				<th>"Who"</th>
				<th>"Why"</th>
				<th>"Before"</th>
				<th>"After"</th>
			</tr>
			{rows}
		</table>
	}
}

fn live_view(changes: &[Change]) -> impl IntoView + use<> {
	let rows = changes
		.iter()
		.map(|change| {
			view! {
				<tr>
					<td>{change.at.to_rfc3339()}</td>
					<td>{change.what.clone()}</td>
				</tr>
			}
		})
		.collect_view();
	view! {
		<p>
			"The changes this viewer hears of, newest first, since the studio began following \
			 them for this viewer. The page reloads every two seconds."
		</p>
		<table>
			<tr>
				<th>"When"</th>
				<th>"Rows"</th>
			</tr>
			{rows}
		</table>
	}
}
