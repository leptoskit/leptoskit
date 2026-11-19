//! Sketchnote, a notebook page where drawings sit in the flow of the text:
//! the worked example for the canvist editor.
//!
//! Some things are quicker to draw than to describe: the line of a ridge,
//! where a path leaves the trees, the spot a bird was hunting. The page here
//! is one editor that holds both. Paragraphs wrap as they do anywhere, and
//! between them sit sketches drawn with a mouse, a pen, or a finger, each
//! stroke one step of undo. Everything is saved in one document.
//!
//! The app owns the editor through a [`CanvistState`] handle and shows it
//! with a `CanvistEditor` element. The document never passes through the
//! screen: the buttons ask the handle for a change, and the status line
//! hears about changes and counts what the saved page holds.
//!
//! ```
//! use sketchnote_example::Notebook;
//! use sketchnote_example::Summary;
//!
//! let notebook = Notebook::open();
//! assert_eq!(notebook.note.insert_drawing(), Ok(true));
//! # notebook.note.dispose();
//!
//! let saved = Summary::of(sketchnote_example::PAGE);
//! assert_eq!(saved.sketches, 1);
//! ```
//!
//! The editor draws itself on the web only for now. A window on another
//! platform shows the rest of the screen around an empty view.

use std::fmt;

use leptoskit::prelude::*;
use leptoskit_canvist::CanvistState;
use leptoskit_canvist::Extensions;
use leptoskit_canvist::Format;
use leptoskit_canvist::Pen;

/// The page the notebook opens on, as the editor saves one: the JSON that
/// `export(Format::Json)` writes, with one sketch already drawn.
pub const PAGE: &str = include_str!("../fixtures/page.json");

/// A colour to draw in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colour {
	/// What the colour is called, which is its button's name.
	pub name: &'static str,
	/// Red, green, and blue, or `None` for the colour of the text, which
	/// follows the page between light and dark.
	pub rgb: Option<(u8, u8, u8)>,
	/// The class that paints its button.
	swatch: &'static str,
}

/// The colours on offer. The first is the pen a new notebook holds.
pub const COLOURS: [Colour; 4] = [
	Colour {
		name: "Ink",
		rgb: None,
		swatch: "bg-stone-900",
	},
	Colour {
		name: "Rose",
		rgb: Some((0xe1, 0x1d, 0x48)),
		swatch: "bg-rose-600",
	},
	Colour {
		name: "Sky",
		rgb: Some((0x02, 0x84, 0xc7)),
		swatch: "bg-sky-600",
	},
	Colour {
		name: "Moss",
		rgb: Some((0x4d, 0x7c, 0x0f)),
		swatch: "bg-lime-700",
	},
];

/// The nibs on offer, by name and width. A sketch is 1000 units across
/// however wide the page is, so a line keeps its weight on a phone and on a
/// wide screen.
pub const NIBS: [(&str, f32); 2] = [("Fine", 4.0), ("Marker", 14.0)];

/// The pen for a colour and a nib, by their places in [`COLOURS`] and
/// [`NIBS`].
///
/// ```
/// use leptoskit_canvist::Pen;
///
/// assert_eq!(sketchnote_example::pen(0, 0), Pen::new().width(4.0));
/// assert_eq!(
/// 	sketchnote_example::pen(1, 1),
/// 	Pen::new().color(0xe1, 0x1d, 0x48).width(14.0)
/// );
/// ```
#[must_use]
pub fn pen(colour: usize, nib: usize) -> Pen {
	let pen = Pen::new().width(NIBS[nib % NIBS.len()].1);

	match COLOURS[colour % COLOURS.len()].rgb {
		Some((red, green, blue)) => pen.color(red, green, blue),
		None => pen,
	}
}

/// What a page holds, for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
	/// Words in every paragraph.
	pub words: usize,
	/// Sketches between them.
	pub sketches: usize,
	/// Strokes in every sketch.
	pub strokes: usize,
}

impl Summary {
	/// Count what a saved page holds. `json` is what the editor exports as
	/// [`Format::Json`]: a tree of nodes, each with a `type`, where a `text`
	/// node carries words and an `ink` node carries a sketch's strokes.
	///
	/// ```
	/// use sketchnote_example::Summary;
	///
	/// let page = r#"{"type":"doc","content":[
	/// 	{"type":"paragraph","content":[{"type":"text","text":"Two tops"}]},
	/// 	{"type":"ink","attrs":{"strokes":[{"points":[0.0,0.0,10.0,10.0]}]}}
	/// ]}"#;
	///
	/// assert_eq!(Summary::of(page).to_string(), "2 words, 1 sketch, 1 stroke");
	/// ```
	#[must_use]
	pub fn of(json: &str) -> Self {
		let mut summary = Self::default();

		if let Ok(page) = serde_json::from_str(json) {
			summary.count(&page);
		}

		summary
	}

	fn count(&mut self, node: &serde_json::Value) {
		match node["type"].as_str() {
			Some("text") => {
				self.words += node["text"]
					.as_str()
					.map_or(0, |text| text.split_whitespace().count());
			}
			Some("ink") => {
				self.sketches += 1;
				self.strokes += node["attrs"]["strokes"].as_array().map_or(0, Vec::len);
			}
			_ => {}
		}

		for child in node["content"].as_array().into_iter().flatten() {
			self.count(child);
		}
	}
}

impl fmt::Display for Summary {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		let counted = |count: usize, one: &str, many: &str| {
			format!("{count} {}", if count == 1 { one } else { many })
		};

		write!(
			formatter,
			"{}, {}, {}",
			counted(self.words, "word", "words"),
			counted(self.sketches, "sketch", "sketches"),
			counted(self.strokes, "stroke", "strokes"),
		)
	}
}

/// A notebook: the editor that holds its page, and what the status line
/// says about it.
#[derive(Clone, Copy, Debug)]
pub struct Notebook {
	/// The editor. The screen gives its id to a `CanvistEditor` element, and
	/// the buttons ask it for changes.
	pub note: CanvistState,
	/// What the page held when it was last counted.
	pub summary: RwSignal<Summary>,
}

impl Notebook {
	/// A notebook open on [`PAGE`], with the first colour and nib in hand.
	///
	/// # Panics
	///
	/// If [`PAGE`] is not a document the editor can read, which the tests
	/// rule out.
	#[must_use]
	pub fn open() -> Self {
		// Paragraphs, and with `ink` the sketches between them.
		let note = CanvistState::new(Extensions::paragraphs().ink());
		note.load(Format::Json, PAGE)
			.unwrap_or_else(|error| panic!("the page the example ships is a document: {error}"));
		note.set_pen(pen(0, 0));
		note.set_placeholder("Write a line, or add a sketch");

		Self {
			note,
			summary: RwSignal::new(Summary::of(PAGE)),
		}
	}

	/// Read the page as it is now and count it again.
	///
	/// Saving works the same way: export the document when a change says
	/// there is something new. The export is a future because an editor in
	/// a native app has to be asked across a bridge; on the web the answer
	/// is ready at once.
	pub async fn recount(self) {
		if let Ok(json) = self.note.export(Format::Json).await {
			self.summary.set(Summary::of(&json));
		}
	}
}

/// The notebook's screen: what it is for, the pens, the page, and a count of
/// what the page holds.
///
/// Nothing here re-renders while someone writes or draws. The buttons call
/// the handle, the two marks under the chosen colour and nib follow their
/// signals in place, and the editor paints its own canvas.
#[component]
pub fn Sketchnote(notebook: Notebook) -> Element {
	let note = notebook.note;
	let colour = remember(|| 0_usize);
	let nib = remember(|| 0_usize);
	let colours = COLOURS
		.iter()
		.enumerate()
		.map(|(index, choice)| {
			view! {
				<View class="items-center gap-1">
					<Button
						aria_label=format!("{} pen", choice.name)
						class=format!("w-9 h-9 rounded-full {}", choice.swatch)
						on:press=move || {
							colour.set(index);
							note.set_pen(pen(index, nib.get_untracked()));
						}
					/>
					<View
						class="w-5 h-1 rounded-full"
						background=move || mark(colour.get() == index)
					/>
				</View>
			}
		})
		.collect::<Fragment>();
	let nibs = NIBS
		.iter()
		.enumerate()
		.map(|(index, (name, _))| {
			view! {
				<View class="items-center gap-1">
					<Button
						class="px-3 h-9 rounded-lg text-sm font-semibold bg-white text-stone-900"
						on:press=move || {
							nib.set(index);
							note.set_pen(pen(colour.get_untracked(), index));
						}
					>
						{*name}
					</Button>
					<View
						class="w-5 h-1 rounded-full"
						background=move || mark(nib.get() == index)
					/>
				</View>
			}
		})
		.collect::<Fragment>();

	view! {
		<View class="flex-col gap-4 p-6 w-full max-w-3xl mx-auto">
			<Heading aria_level=1 class="text-3xl font-bold text-stone-900">"Sketchnote"</Heading>
			<Text class="text-base text-stone-600">
				"Write what happened, and draw the part that words are slow at. Sketches sit between the paragraphs and are saved with them."
			</Text>
			<View class="flex-row flex-wrap items-start gap-3">
				<Button
					class="px-4 h-9 rounded-lg text-sm font-semibold bg-stone-900 text-white"
					on:press={move || {
						// The sketch goes in at the caret. `Ok(false)` means it
						// could not go there, and there is nothing to report.
						let _ = note.insert_drawing();
					}}
				>
					"Add a sketch"
				</Button>
				{colours}
				{nibs}
				<Button
					class="px-3 h-9 rounded-lg text-sm font-semibold bg-white text-stone-900"
					on:press=move || {
						note.undo();
					}
				>
					"Undo"
				</Button>
				<Button
					class="px-3 h-9 rounded-lg text-sm font-semibold bg-white text-stone-900"
					on:press=move || {
						note.redo();
					}
				>
					"Redo"
				</Button>
			</View>
			<CanvistEditor
				editor=note.id()
				aria_label="Notebook page"
				class="w-full min-h-96 p-5 rounded-xl bg-white text-lg text-stone-900"
			/>
			<Text class="text-sm text-stone-500">{move || notebook.summary.get().to_string()}</Text>
		</View>
	}
}

/// The bar under a pen or nib: dark under the chosen one, and the colour of
/// the page under the rest.
fn mark(chosen: bool) -> Color {
	if chosen {
		Color::rgba(0x1c, 0x19, 0x17, 0xff)
	} else {
		Color::rgba(0, 0, 0, 0)
	}
}

/// Put the notebook in the page's body, and count the page again after
/// every change to it. Returns the notebook, so a test can ask it what it
/// holds.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub fn mount() -> Notebook {
	let notebook = Notebook::open();
	notebook.note.on_change(move |change| {
		// A caret that moved changes nothing worth counting.
		if change.doc_changed {
			wasm_bindgen_futures::spawn_local(notebook.recount());
		}
	});
	leptos::mount::mount_to_body(move || {
		leptoskit::web::screen(move || view! { <Sketchnote notebook=notebook /> })
	});

	notebook
}

/// Mounts the notebook when the browser loads the Wasm module. `leptoskit
/// dev` builds it with the `csr` feature and serves it from
/// `web/index.html`.
#[cfg(all(target_arch = "wasm32", feature = "csr"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	mount();
}
