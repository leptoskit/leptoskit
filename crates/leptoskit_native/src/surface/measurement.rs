//! Cached text and native-control measurement for the existing layout tree.

use leptoskit_yoga::NodeId;

use super::NativeSurface;
use super::TextMeasureCache;
use super::has_native_chrome;
use super::measured_content;
use crate::backend::NativeBackend;
use crate::props::NativeKind;
use crate::props::NativeProps;

impl<B: NativeBackend> NativeSurface<B> {
	/// Replaces cached text metrics with the backend's current measurer and
	/// lays out again without replacing native widgets or event handlers.
	///
	/// Call after a host changes its font environment, such as display DPI or
	/// system text scale. Each returned measurer must still have immutable
	/// metrics: changing its results behind the layout cache is unsupported.
	pub fn refresh_text_measurer(&mut self) {
		self.measurer = self.backend.text_measurer();
		let mut pending: Vec<_> = self.root.into_iter().collect();

		while let Some(id) = pending.pop() {
			pending.extend_from_slice(&self.shadows[id.index()].children);

			if let Some(cache) = &mut self.shadows[id.index()].text_cache {
				cache.sizes = [None; 2];
				self.yoga.clear_measure_cache(id);
			}
		}

		self.commit_layout();
	}

	#[inline]
	pub(super) fn attach_text_measure(
		&mut self,
		id: NodeId,
		kind: NativeKind,
		props: &NativeProps,
	) {
		let style = measured_style(kind, props);
		let native_chrome = has_native_chrome(kind, props);
		self.shadows[id.index()].text_cache = Some(TextMeasureCache::new(style, native_chrome));
		self.yoga.set_measured(id, true);
	}

	/// Refreshes cached text only when the existing measurement inputs change.
	pub(super) fn update_text_measure(
		&mut self,
		id: NodeId,
		kind: NativeKind,
		content_changed: bool,
	) {
		let has_content = measured_content(kind, &self.shadows[id.index()].props).is_some();

		if !has_content {
			if self.shadows[id.index()].text_cache.take().is_some() {
				self.yoga.set_measured(id, false);
			}

			return;
		}

		let style = measured_style(kind, &self.shadows[id.index()].props);
		let native_chrome = has_native_chrome(kind, &self.shadows[id.index()].props);

		if let Some(cache) = &mut self.shadows[id.index()].text_cache {
			// Colors and the like do not change the text's size, so only a new
			// string, font, or native chrome invalidates layout.
			if content_changed || cache.style != style || cache.native_chrome != native_chrome {
				cache.style = style;
				cache.native_chrome = native_chrome;
				cache.sizes = [None; 2];
				self.yoga.clear_measure_cache(id);
			}
		} else {
			self.shadows[id.index()].text_cache = Some(TextMeasureCache::new(style, native_chrome));
			self.yoga.set_measured(id, true);
		}
	}
}

/// The style an element's text is measured in. Text fields keep their
/// platform's single-line height, whatever line height they inherit.
pub(super) fn measured_style(kind: NativeKind, props: &NativeProps) -> crate::props::TextStyle {
	let mut style = props.text_style();

	if matches!(kind, NativeKind::TextInput | NativeKind::PasswordInput) {
		style.line_height = None;
	}

	style
}
