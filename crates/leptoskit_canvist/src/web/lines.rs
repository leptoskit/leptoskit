use canvist_headless::canvist_core::Caret;
use canvist_headless::canvist_core::LineEdge;
use canvist_headless::canvist_core::LineNavigator;
use canvist_headless::canvist_core::NavigationError;
use canvist_headless::canvist_core::VerticalDirection;
use canvist_headless::canvist_core::VerticalMotion;
use canvist_headless::canvist_model::Document;
use canvist_layout::DocumentLayout;
use canvist_layout::TextMeasurer;

pub struct HostLines<'a> {
	pub layout: &'a mut DocumentLayout,
	pub measurer: &'a mut dyn TextMeasurer,
}

impl<'a> HostLines<'a> {
	pub fn new(layout: &'a mut DocumentLayout, measurer: &'a mut dyn TextMeasurer) -> Self {
		Self { layout, measurer }
	}
}

impl LineNavigator for HostLines<'_> {
	fn vertical(
		&mut self,
		doc: &Document,
		from: Caret,
		direction: VerticalDirection,
		goal_x: Option<f32>,
	) -> Result<Option<VerticalMotion>, NavigationError> {
		let mut navigator = self.layout.navigator(doc, self.measurer);
		LineNavigator::vertical(&mut navigator, doc, from, direction, goal_x)
	}

	fn line_edge(
		&mut self,
		doc: &Document,
		at: Caret,
		edge: LineEdge,
	) -> Result<Caret, NavigationError> {
		let mut navigator = self.layout.navigator(doc, self.measurer);
		LineNavigator::line_edge(&mut navigator, doc, at, edge)
	}
}