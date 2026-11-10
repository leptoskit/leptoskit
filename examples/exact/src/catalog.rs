//! The example catalogue.

use leptoskit::prelude::*;

use crate::screens;

/// One Exact app, rebuilt as a leptoskit screen.
pub struct Example {
	/// The command name, matching Exact's app name.
	pub name: &'static str,
	/// The heading the screen shows.
	pub title: &'static str,
	/// What the screen is for, in one line.
	pub teaches: &'static str,
	/// Builds the screen.
	pub render: fn() -> Element,
}

/// Every example, in the order Exact's guide introduces them.
pub const EXAMPLES: &[Example] = &[
	Example {
		name: "hello",
		title: "Hello",
		teaches: "The screen a new app starts from",
		render: screens::Hello,
	},
	Example {
		name: "todo",
		title: "Todo",
		teaches: "Add, finish, and delete, from Exact's guide",
		render: screens::Todo,
	},
	Example {
		name: "shelf",
		title: "Shelf",
		teaches: "A recipe app that names no colour and no font size",
		render: screens::Shelf,
	},
	Example {
		name: "caltrain",
		title: "Caltrain",
		teaches: "Stations, departures, and a picked stop",
		render: screens::Caltrain,
	},
	Example {
		name: "weatherlight",
		title: "Weatherlight",
		teaches: "Weather that follows the hour you pick",
		render: screens::Weatherlight,
	},
	Example {
		name: "fieldnotes",
		title: "Fieldnotes",
		teaches: "A notebook with pins",
		render: screens::Fieldnotes,
	},
	Example {
		name: "recorder",
		title: "Recorder",
		teaches: "A microphone, ready and then recording",
		render: screens::Recorder,
	},
	Example {
		name: "spark",
		title: "Spark",
		teaches: "A card you throw to reveal the next",
		render: screens::Spark,
	},
	Example {
		name: "expose",
		title: "Expose",
		teaches: "A lock screen that opens onto a home screen",
		render: screens::Expose,
	},
	Example {
		name: "calendar",
		title: "Calendar",
		teaches: "A month and the day you pick",
		render: screens::Calendar,
	},
	Example {
		name: "sparkline",
		title: "Sparkline",
		teaches: "A market list with a chart on each row",
		render: screens::Sparkline,
	},
	Example {
		name: "photo-editor",
		title: "Photo editor",
		teaches: "Rotate and crop",
		render: screens::PhotoEditor,
	},
	Example {
		name: "video-player",
		title: "Video player",
		teaches: "A clip that shrinks when the keyboard opens",
		render: screens::VideoPlayer,
	},
	Example {
		name: "exact-live",
		title: "Exact Live",
		teaches: "Photos, a crew chat, and a runbook",
		render: screens::ExactLive,
	},
	Example {
		name: "still",
		title: "Still",
		teaches: "Reorder a list, and a sheet of records",
		render: screens::Still,
	},
	Example {
		name: "reflow",
		title: "Reflow",
		teaches: "Typography studies",
		render: screens::Reflow,
	},
	Example {
		name: "textflow",
		title: "Textflow",
		teaches: "Text beside a shape",
		render: screens::Textflow,
	},
	Example {
		name: "markdown",
		title: "Markdown",
		teaches: "A reader for one document",
		render: screens::Markdown,
	},
	Example {
		name: "conduit",
		title: "Conduit",
		teaches: "A magazine feed and one article",
		render: screens::Conduit,
	},
	Example {
		name: "svg-gallery",
		title: "SVG gallery",
		teaches: "SVG features, one page at a time",
		render: screens::SvgGallery,
	},
	Example {
		name: "canvas-gallery",
		title: "Canvas gallery",
		teaches: "Canvas 2D pages, drawn as views",
		render: screens::CanvasGallery,
	},
	Example {
		name: "messages",
		title: "Messages",
		teaches: "A chat with one scripted reply",
		render: screens::Messages,
	},
	Example {
		name: "typetour",
		title: "Type tour",
		teaches: "A type specimen",
		render: screens::TypeTour,
	},
	Example {
		name: "map-demo",
		title: "Map demo",
		teaches: "Pins a map module would draw",
		render: screens::MapDemo,
	},
	Example {
		name: "carousel",
		title: "Carousel",
		teaches: "25,000 cards, windowed",
		render: screens::Carousel,
	},
	Example {
		name: "motion-gallery",
		title: "Motion gallery",
		teaches: "GIF, WebP, and a keyframe",
		render: screens::MotionGallery,
	},
	Example {
		name: "update-lab",
		title: "Update lab",
		teaches: "Replace a running screen",
		render: screens::UpdateLab,
	},
	Example {
		name: "messages-stress",
		title: "Messages stress",
		teaches: "A window into 100,000 messages",
		render: screens::MessagesStress,
	},
	Example {
		name: "completion-storm",
		title: "Completion storm",
		teaches: "128 requests finishing together",
		render: screens::CompletionStorm,
	},
	Example {
		name: "markdown-stress",
		title: "Markdown stress",
		teaches: "The shape of a 4 MiB document",
		render: screens::MarkdownStress,
	},
	Example {
		name: "native-fixture",
		title: "Native fixture",
		teaches: "A native module panel",
		render: screens::NativeFixture,
	},
	Example {
		name: "auth-fixture",
		title: "Auth fixture",
		teaches: "PAR, DPoP, and PKCE",
		render: screens::AuthFixture,
	},
	Example {
		name: "windows-desk",
		title: "Windows desk",
		teaches: "Native controls on a desk",
		render: screens::WindowsDesk,
	},
];

/// The example with this command name.
pub fn find(name: &str) -> Option<&'static Example> {
	EXAMPLES.iter().find(|example| example.name == name)
}

/// The example a page's query names with `screen`, as in `?screen=caltrain`.
///
/// ```
/// assert_eq!(
/// 	exact_examples::screen_in_query("?screen=caltrain").map(|example| example.title),
/// 	Some("Caltrain")
/// );
/// assert!(exact_examples::screen_in_query("?screen=missing").is_none());
/// ```
pub fn screen_in_query(query: &str) -> Option<&'static Example> {
	query
		.trim_start_matches('?')
		.split('&')
		.find_map(|pair| pair.strip_prefix("screen="))
		.and_then(find)
}
