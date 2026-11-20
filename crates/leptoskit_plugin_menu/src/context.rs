//! Context menus (right-click / secondary-click menus on desktop, long-press
//! menus on touch devices).

use crate::Definition;
use crate::Error;
use crate::Item;
use crate::builder::MenuBuilder;
use crate::builder::SubmenuBuilder;

/// Starts a context menu tree. Unlike [`crate::MenuBar`], context menus accept
/// commands, checks, radios, roles, separators, and submenus at the root.
#[must_use]
#[allow(non_snake_case)]
pub fn ContextMenu() -> ContextMenuBuilder {
	ContextMenuBuilder {
		inner: MenuBuilder::default(),
	}
}

/// Logical window-relative coordinates where a context menu should open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextMenuAnchor {
	/// Horizontal coordinate in logical screen/window points.
	pub x: f64,
	/// Vertical coordinate in logical screen/window points.
	pub y: f64,
}

impl ContextMenuAnchor {
	/// Validates finite coordinates.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidDefinition`] when either coordinate is `NaN` or
	/// infinite.
	pub fn new(x: f64, y: f64) -> Result<Self, Error> {
		if x.is_finite() && y.is_finite() {
			Ok(Self { x, y })
		} else {
			Err(Error::InvalidDefinition)
		}
	}
}

/// Validated context menu presentation request.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenuRequest {
	/// Validated menu items to display.
	pub definition: Definition,
	/// Optional anchor point; when `None`, the host opens at the current
	/// pointer location.
	pub anchor: Option<ContextMenuAnchor>,
}

/// Builder for [`ContextMenuRequest`].
#[derive(Debug, Clone, Default)]
pub struct ContextMenuBuilder {
	inner: MenuBuilder,
}

impl ContextMenuBuilder {
	/// Appends one menu item.
	#[must_use]
	pub fn item(mut self, item: impl Into<Item>) -> Self {
		self.inner = self.inner.item(item);
		self
	}

	/// Appends many items in order.
	#[must_use]
	pub fn items(mut self, items: impl IntoIterator<Item = impl Into<Item>>) -> Self {
		self.inner = self.inner.items(items);
		self
	}

	/// Appends a separator.
	#[must_use]
	pub fn separator(mut self) -> Self {
		self.inner = self.inner.separator();
		self
	}

	/// Appends a nested submenu.
	#[must_use]
	pub fn submenu(mut self, submenu: SubmenuBuilder) -> Self {
		self.inner = self.inner.submenu(submenu);
		self
	}

	/// Builds a validated [`ContextMenuRequest`] anchored at the pointer
	/// location.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidDefinition`] or [`Error::InvalidAccelerator`]
	/// if validation fails.
	pub fn build(self) -> Result<ContextMenuRequest, Error> {
		Ok(ContextMenuRequest {
			definition: self.inner.build()?,
			anchor: None,
		})
	}

	/// Builds a validated [`ContextMenuRequest`] at an explicit window point.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidDefinition`] when coordinates are nonfinite or
	/// the menu tree fails validation.
	pub fn build_at(self, x: f64, y: f64) -> Result<ContextMenuRequest, Error> {
		Ok(ContextMenuRequest {
			definition: self.inner.build()?,
			anchor: Some(ContextMenuAnchor::new(x, y)?),
		})
	}
}

/// Host contract for presenting native context menus.
pub trait ContextMenuHost {
	/// Displays `request` on the native UI thread.
	///
	/// # Errors
	///
	/// Returns [`Error::Unavailable`] when the host cannot display a context
	/// menu, or [`Error::Native`] when presentation fails.
	fn present(&mut self, request: ContextMenuRequest) -> Result<(), Error>;
}

/// Recording [`ContextMenuHost`] for tests and custom hosts.
#[derive(Debug, Clone, Default)]
pub struct MemoryContextMenuHost {
	presented: Vec<ContextMenuRequest>,
}

impl MemoryContextMenuHost {
	/// Starts an empty recording host.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Every context menu request presented so far.
	#[must_use]
	pub fn presented(&self) -> &[ContextMenuRequest] {
		&self.presented
	}
}

impl ContextMenuHost for MemoryContextMenuHost {
	fn present(&mut self, request: ContextMenuRequest) -> Result<(), Error> {
		self.presented.push(request);
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Command;
	use crate::MenuRole;
	use crate::Role;

	#[test]
	fn context_menu_validates_items_and_anchor() {
		let request = ContextMenu()
			.item(Command("rename").label("Rename"))
			.separator()
			.item(Role(MenuRole::Copy))
			.build_at(120.0, 240.0)
			.unwrap();
		assert_eq!(request.definition.items().len(), 3);
		assert_eq!(
			request.anchor,
			Some(ContextMenuAnchor { x: 120.0, y: 240.0 })
		);

		let mut host = MemoryContextMenuHost::new();
		host.present(request).unwrap();
		assert_eq!(host.presented().len(), 1);

		assert_eq!(
			ContextMenu()
				.item(Command("ok").label("OK"))
				.build_at(f64::NAN, 0.0),
			Err(Error::InvalidDefinition)
		);
	}
}
