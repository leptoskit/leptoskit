//! Stack, Tabs, Drawer, deep links, and system Back.

use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_router::__private::decode;
use leptoskit_router::ActivationRef;
use leptoskit_router::Href;
use leptoskit_router::NoMatch;
use leptoskit_router::OpenError;
use leptoskit_router::OpenMode;
use leptoskit_router::Pattern;
use leptoskit_router::Render;
use leptoskit_router::Routable;
use leptoskit_router::Route;
use leptoskit_router::RouteUrl;
use leptoskit_router::Router;
use leptoskit_router::Segment;
use leptoskit_router::SystemBack;
use leptoskit_router::apply_activations;
use leptoskit_router::clear_system_back_handler;
use leptoskit_router::dispatch_system_back;
use leptoskit_router::navigators::Drawer;
use leptoskit_router::navigators::DrawerItem;
use leptoskit_router::navigators::IntoNavigator;
use leptoskit_router::navigators::Navigator;
use leptoskit_router::navigators::Presentation;
use leptoskit_router::navigators::Stack;
use leptoskit_router::navigators::StackGesture;
use leptoskit_router::navigators::Tab;
use leptoskit_router::navigators::TabBar;
use leptoskit_router::navigators::TabPlacement;
use leptoskit_router::navigators::Tabs;
use leptoskit_router::navigators::href_matches_root;
use leptoskit_router::route_path_from_link;
use reactive_graph::traits::GetUntracked;

#[derive(Route, Clone, Debug, PartialEq)]
struct NoteRoute {
	note: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum Screen {
	Home,
	Notes,
	Note(NoteRoute),
	Compose,
	Photo,
	You,
	Settings,
}

const HOME: Pattern = Pattern::new(&[]);
const NOTES: Pattern = Pattern::new(&[Segment::Static("notes")]);
const NOTE: Pattern = Pattern::new(&[Segment::Static("notes"), Segment::Param("note")]);
const COMPOSE: Pattern = Pattern::new(&[Segment::Static("compose")]);
const PHOTO: Pattern = Pattern::new(&[Segment::Static("photos"), Segment::Static("1")]);
const YOU: Pattern = Pattern::new(&[Segment::Static("you")]);
const SETTINGS: Pattern = Pattern::new(&[Segment::Static("settings")]);

impl Routable for Screen {
	fn from_url(url: &RouteUrl) -> Result<Self, NoMatch> {
		if let Some(captures) = NOTE.capture(url.segments()) {
			return decode(&captures, url).map(Self::Note).map_err(|_| NoMatch);
		}
		if COMPOSE.capture(url.segments()).is_some() {
			return Ok(Self::Compose);
		}
		if PHOTO.capture(url.segments()).is_some() {
			return Ok(Self::Photo);
		}
		if NOTES.capture(url.segments()).is_some() {
			return Ok(Self::Notes);
		}
		if YOU.capture(url.segments()).is_some() {
			return Ok(Self::You);
		}
		if SETTINGS.capture(url.segments()).is_some() {
			return Ok(Self::Settings);
		}
		HOME.capture(url.segments())
			.map(|_| Self::Home)
			.ok_or(NoMatch)
	}

	fn href(&self) -> Href {
		match self {
			Self::Home => HOME.static_href(),
			Self::Notes => NOTES.static_href(),
			Self::Note(note) => NOTE.href(note),
			Self::Compose => COMPOSE.static_href(),
			Self::Photo => PHOTO.static_href(),
			Self::You => YOU.static_href(),
			Self::Settings => SETTINGS.static_href(),
		}
	}
}

impl Render for Screen {
	fn render(&self, _navigation: &Router<Self>) -> Element {
		Text(match self {
			Self::Home => "Home".to_owned(),
			Self::Notes => "Notes".to_owned(),
			Self::Note(note) => format!("Note {}", note.note),
			Self::Compose => "Compose".to_owned(),
			Self::Photo => "Photo".to_owned(),
			Self::You => "You".to_owned(),
			Self::Settings => "Settings".to_owned(),
		})
	}
}

#[test]
fn href_matching_treats_root_as_exact() {
	assert!(href_matches_root(
		&Screen::Home.href(),
		&Screen::Home.href()
	));
	assert!(!href_matches_root(
		&Screen::Notes.href(),
		&Screen::Home.href()
	));
	assert!(href_matches_root(
		&Screen::Note(NoteRoute { note: 42 }).href(),
		&Screen::Notes.href()
	));
	assert!(href_matches_root(
		&Screen::parse("/notes?x=1").unwrap().href(),
		&Screen::Notes.href()
	));
}

#[test]
fn tabs_select_by_href_prefix_and_replace_on_press() {
	let tabs = Tabs::new([
		Tab::root(Screen::Home).title("Today"),
		Tab::root(Screen::Notes).title("Notes"),
		Tab::root(Screen::You).title("You"),
	]);
	assert_eq!(
		tabs.selected(&Screen::Note(NoteRoute { note: 3 }))
			.map(Tab::label),
		Some("Notes")
	);
	assert_eq!(tabs.selected(&Screen::Home).map(Tab::label), Some("Today"));

	let router = Router::<Screen>::new(Screen::Home);
	router.navigate(Screen::Note(NoteRoute { note: 1 }));
	assert!(router.can_go_back());

	let shell =
		tabs.clone()
			.into_navigator()
			.shell(Text("body".to_owned()), &router, &router.current());

	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&shell);
	let notes = surface
		.root()
		.and_then(|root| find_test_id(&surface, root, "tab-notes"))
		.expect("notes tab");
	surface.dispatch_event(notes, &NativeEvent::Press);
	assert_eq!(router.current(), Screen::Notes);
	// Replace drops the note entry, so Back returns to Home — not the note.
	assert_eq!(router.previous(), Some(Screen::Home));
}

#[test]
fn tabs_draw_a_custom_bar_that_still_replaces() {
	let tabs = Tabs::new([
		Tab::root(Screen::Home).title("Today"),
		Tab::root(Screen::Notes).title("Notes"),
	])
	.bar(|bar: &TabBar<'_, Screen>| {
		let links = bar.items().map(|(tab, selected)| {
			let mark = if selected { "selected" } else { "idle" };
			bar.link(tab, Text(format!("{} {mark}", tab.label())))
		});
		View().test_id("custom-bar").children(links)
	});
	assert!(format!("{tabs:?}").contains("bar: true"));

	let router = Router::<Screen>::new(Screen::Home);
	router.navigate(Screen::Note(NoteRoute { note: 1 }));
	let shell = tabs
		.into_navigator()
		.shell(Text("body".to_owned()), &router, &router.current());
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&shell);
	let root = surface.root().unwrap();
	assert!(find_test_id(&surface, root, "custom-bar").is_some());
	assert!(find_test_id(&surface, root, "tabs-bar").is_none());
	let notes = find_test_id(&surface, root, "tab-notes").expect("notes link");
	surface.dispatch_event(notes, &NativeEvent::Press);
	assert_eq!(router.current(), Screen::Notes);
	assert_eq!(router.previous(), Some(Screen::Home));
}

#[test]
fn stack_draws_a_custom_surface_for_sheets() {
	let stack = Stack::new()
		.present(Presentation::Sheet, |route| {
			matches!(route, Screen::Compose)
		})
		.surface(|presentation, children| {
			assert_eq!(presentation, Presentation::Sheet);
			View().test_id("paper").child(children)
		});
	assert!(format!("{stack:?}").contains("surface: true"));

	let router = Router::<Screen>::new(Screen::Home);
	router.navigate(Screen::Compose);
	let shell = stack.shell(Text("compose".to_owned()), &router, &Screen::Compose);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&shell);
	let root = surface.root().unwrap();
	// The layer keeps the stack's test id, so tests that find the sheet still
	// do.
	assert!(find_test_id(&surface, root, "stack-sheet").is_some());
	assert!(find_test_id(&surface, root, "stack-overlay").is_some());
}

#[test]
fn stack_presents_modals_over_the_previous_route() {
	let stack = Stack::new()
		.gesture(StackGesture::EdgeSwipe)
		.present(Presentation::Modal, |route| matches!(route, Screen::Photo))
		.present(Presentation::Sheet, |route| {
			matches!(route, Screen::Compose)
		});

	assert_eq!(stack.presentation_for(&Screen::Photo), Presentation::Modal);
	assert_eq!(
		stack.presentation_for(&Screen::Compose),
		Presentation::Sheet
	);
	assert_eq!(stack.presentation_for(&Screen::Home), Presentation::Push);

	let router = Router::<Screen>::new(Screen::Home);
	router.navigate(Screen::Photo);
	let shell = stack.shell(Text("photo".to_owned()), &router, &Screen::Photo);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&shell);
	assert!(
		find_test_id(&surface, surface.root().unwrap(), "stack-overlay").is_some(),
		"modal paints over the previous route"
	);
	assert!(find_test_id(&surface, surface.root().unwrap(), "stack-modal").is_some());
	assert!(find_test_id(&surface, surface.root().unwrap(), "stack-edge-hit").is_some());
}

#[test]
fn drawer_opens_navigates_and_closes() {
	let drawer = Drawer::new([
		DrawerItem::root(Screen::Home).title("Home"),
		DrawerItem::root(Screen::Settings).title("Settings"),
	]);
	let router = Router::<Screen>::new(Screen::Home);
	assert!(!router.drawer_open().get_untracked());

	let shell = drawer.shell(Text("body".to_owned()), &router, &Screen::Home);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&shell);
	let toggle = find_test_id(&surface, surface.root().unwrap(), "drawer-toggle").unwrap();
	surface.dispatch_event(toggle, &NativeEvent::Press);
	assert!(router.drawer_open().get_untracked());

	let shell = drawer.shell(Text("body".to_owned()), &router, &Screen::Home);
	surface.set_root(&shell);
	let settings = find_test_id(&surface, surface.root().unwrap(), "drawer-settings").unwrap();
	surface.dispatch_event(settings, &NativeEvent::Press);
	assert_eq!(router.current(), Screen::Settings);
	assert!(!router.drawer_open().get_untracked());
}

#[test]
fn open_url_parses_custom_schemes_and_https_paths() {
	assert_eq!(
		route_path_from_link("myapp://notes/42?x=1").as_deref(),
		Some("/notes/42?x=1")
	);
	assert_eq!(
		route_path_from_link("myapp:///compose").as_deref(),
		Some("/compose")
	);
	assert_eq!(
		route_path_from_link("https://example.com/notes/7").as_deref(),
		Some("/notes/7")
	);
	assert_eq!(route_path_from_link("/you").as_deref(), Some("/you"));
	assert_eq!(route_path_from_link(""), None);

	let router = Router::<Screen>::new(Screen::Home);
	router.open_url("myapp://notes/9").unwrap();
	assert_eq!(router.current(), Screen::Note(NoteRoute { note: 9 }));
	assert!(router.can_go_back());

	router
		.open_url_with("myapp://you", OpenMode::Replace)
		.unwrap();
	assert_eq!(router.current(), Screen::You);

	assert!(matches!(
		router.open_url("myapp://missing"),
		Err(OpenError::NoMatch(_))
	));
}

#[test]
fn deep_link_activations_drive_the_router() {
	let router = Router::<Screen>::new(Screen::Home);
	let results = apply_activations(
		&router,
		[
			ActivationRef::Url("myapp://settings"),
			ActivationRef::File(std::path::Path::new("/tmp/note.md")),
		],
		OpenMode::Navigate,
	);
	assert_eq!(results, vec![Ok(())]);
	assert_eq!(router.current(), Screen::Settings);
}

#[test]
fn system_back_consumes_history_then_propagates() {
	clear_system_back_handler();
	assert_eq!(dispatch_system_back(), SystemBack::Propagate);

	let router = Router::<Screen>::new(Screen::Home);
	router.bind_system_back();
	assert_eq!(dispatch_system_back(), SystemBack::Propagate);

	router.navigate(Screen::Notes);
	assert_eq!(dispatch_system_back(), SystemBack::Consumed);
	assert_eq!(router.current(), Screen::Home);
	assert_eq!(dispatch_system_back(), SystemBack::Propagate);

	clear_system_back_handler();
}

#[test]
fn previous_peeks_memory_history() {
	let router = Router::<Screen>::new(Screen::Home);
	assert_eq!(router.previous(), None);
	router.navigate(Screen::Notes);
	assert_eq!(router.previous(), Some(Screen::Home));
	assert!(router.back());
	assert_eq!(router.previous(), None);
}

#[test]
fn route_path_covers_https_query_and_hostless_schemes() {
	assert_eq!(
		route_path_from_link("https://example.com/?q=1").as_deref(),
		Some("/?q=1")
	);
	assert_eq!(
		route_path_from_link("https://example.com?q=2").as_deref(),
		Some("/?q=2")
	);
	// No host and only `/` → the synthetic app root.
	assert_eq!(route_path_from_link("myapp:///").as_deref(), Some("/"));
	assert_eq!(route_path_from_link("myapp:").as_deref(), Some("/"));
	assert!(route_path_from_link("not a url").is_none());
	assert!(route_path_from_link("https://example.com/\u{0001}").is_none());
}

#[test]
fn stack_default_debug_push_sheet_and_edge_swipe() {
	let defaulted = Stack::<Screen>::default();
	assert_eq!(
		defaulted.presentation_for(&Screen::Home),
		Presentation::Push
	);
	assert!(format!("{defaulted:?}").contains("Stack"));

	let stack = Stack::new()
		.gesture(StackGesture::None)
		.present(Presentation::Sheet, |route| {
			matches!(route, Screen::Compose)
		});

	// Push shell with no gesture chrome.
	let router = Router::<Screen>::new(Screen::Home);
	let push = stack.shell(Text("home".to_owned()), &router, &Screen::Home);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&push);
	assert!(
		find_test_id(&surface, surface.root().unwrap(), "stack-edge-hit").is_none(),
		"no history means no edge hit"
	);

	// Edge swipe with nothing to pop returns the children unchanged.
	let idle_swipe = Stack::new().gesture(StackGesture::EdgeSwipe);
	let idle = idle_swipe.shell(Text("home".to_owned()), &router, &Screen::Home);
	surface.set_root(&idle);
	assert!(find_test_id(&surface, surface.root().unwrap(), "stack-edge-hit").is_none());
	assert!(find_test_id(&surface, surface.root().unwrap(), "stack-edge-swipe").is_none());

	// Stack::into_navigator (not only Navigator::into_navigator).
	let _ = Stack::<Screen>::new().into_navigator();

	// Sheet with history underneath.
	router.navigate(Screen::Compose);
	let sheet = stack.shell(Text("compose".to_owned()), &router, &Screen::Compose);
	surface.set_root(&sheet);
	assert!(find_test_id(&surface, surface.root().unwrap(), "stack-sheet").is_some());
	assert!(find_test_id(&surface, surface.root().unwrap(), "stack-overlay").is_some());

	// Sheet with no previous route (cold open).
	let cold = Router::<Screen>::new(Screen::Compose);
	let cold_sheet = stack.shell(Text("compose".to_owned()), &cold, &Screen::Compose);
	surface.set_root(&cold_sheet);
	assert!(find_test_id(&surface, surface.root().unwrap(), "stack-sheet").is_some());
	assert!(find_test_id(&surface, surface.root().unwrap(), "stack-overlay").is_none());

	// Edge swipe pops when history exists.
	let swipe = Stack::new().gesture(StackGesture::EdgeSwipe);
	let router = Router::<Screen>::new(Screen::Home);
	router.navigate(Screen::Notes);
	let shell = swipe.shell(Text("notes".to_owned()), &router, &Screen::Notes);
	surface.set_root(&shell);
	let hit = find_test_id(&surface, surface.root().unwrap(), "stack-edge-hit").unwrap();
	surface.dispatch_event(hit, &NativeEvent::Press);
	assert_eq!(router.current(), Screen::Home);
}

#[test]
fn navigator_enum_shells_every_variant() {
	let router = Router::<Screen>::new(Screen::Home);
	let body = Text("body".to_owned());
	let mut surface = NativeSurface::new(HeadlessBackend::new());

	let stack = Navigator::Stack(Stack::new()).into_navigator();
	surface.set_root(&stack.shell(body.clone(), &router, &Screen::Home));
	assert!(surface.root().is_some());

	let tabs = Tabs::new([Tab::root(Screen::Home).title("Home")]).placement(TabPlacement::Top);
	assert_eq!(tabs.items().len(), 1);
	assert_eq!(tabs.items()[0].route(), &Screen::Home);
	assert_eq!(tabs.items()[0].label(), "Home");
	let top = tabs
		.clone()
		.into_navigator()
		.shell(body.clone(), &router, &Screen::Home);
	surface.set_root(&top);
	assert!(find_test_id(&surface, surface.root().unwrap(), "tabs-shell").is_some());

	let drawer = Navigator::Drawer(Drawer::new([DrawerItem::root(Screen::Home).title("Home")]));
	surface.set_root(&drawer.shell(body, &router, &Screen::Home));
	assert!(find_test_id(&surface, surface.root().unwrap(), "drawer-shell").is_some());
}

#[test]
fn drawer_accessors_and_selected_row() {
	let item = DrawerItem::root(Screen::Settings).title("Settings");
	assert_eq!(item.route(), &Screen::Settings);
	assert_eq!(item.label(), "Settings");

	let drawer = Drawer::new([DrawerItem::root(Screen::Home).title("Home"), item]);
	assert_eq!(drawer.items().len(), 2);
	assert_eq!(
		drawer.selected(&Screen::Settings).map(DrawerItem::label),
		Some("Settings")
	);
	assert!(drawer.selected(&Screen::You).is_none());

	let _ = drawer.into_navigator();
}

#[test]
fn href_root_matches_query_only_home() {
	assert!(href_matches_root(
		&Screen::parse("/?x=1").unwrap().href(),
		&Screen::Home.href()
	));
}

fn find_test_id(
	surface: &NativeSurface<HeadlessBackend>,
	node: leptoskit_yoga::NodeId,
	id: &str,
) -> Option<leptoskit_yoga::NodeId> {
	if surface.props(node).test_id() == Some(id) {
		return Some(node);
	}
	for child in surface.children(node) {
		if let Some(found) = find_test_id(surface, *child, id) {
			return Some(found);
		}
	}
	None
}
