//! iOS and iPadOS menu bar (`UIMenuBuilder` / `UIMenu`) and hardware keyboard
//! command (`UIKeyCommand`) definitions.
//!
//! On iPadOS (and iOS with a hardware keyboard or Mac Catalyst), UIKit exposes
//! a system menu bar built via `UIResponder.buildMenu(with:)` and top-level
//! `UIKeyCommand` shortcuts. This module gives apps a validated,
//! platform-native Rust model that compiles on every host and maps 1:1 to
//! UIKit's standard menu identifiers (`application`, `file`, `edit`, `format`,
//! `view`, `window`, `help`).

use std::collections::HashSet;

use crate::Definition;
use crate::Error;
use crate::Item;
use crate::MAX_ITEMS;
use crate::valid_label;

/// Standard `UIMenu.Identifier` placement in the iOS / iPadOS menu bar.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IosMenuPlacement {
	/// Application menu (`UIMenu.Identifier.application`).
	Application,
	/// File menu (`UIMenu.Identifier.file`).
	File,
	/// Edit menu (`UIMenu.Identifier.edit`).
	Edit,
	/// Format menu (`UIMenu.Identifier.format`).
	Format,
	/// View menu (`UIMenu.Identifier.view`).
	View,
	/// Window menu (`UIMenu.Identifier.window`).
	Window,
	/// Help menu (`UIMenu.Identifier.help`).
	Help,
	/// Custom top-level menu inserted after the named system identifier.
	Custom(String),
}

impl IosMenuPlacement {
	/// UIKit `UIMenu.Identifier` wire name.
	#[must_use]
	pub fn identifier(&self) -> &str {
		match self {
			Self::Application => "com.apple.menu.application",
			Self::File => "com.apple.menu.file",
			Self::Edit => "com.apple.menu.edit",
			Self::Format => "com.apple.menu.format",
			Self::View => "com.apple.menu.view",
			Self::Window => "com.apple.menu.window",
			Self::Help => "com.apple.menu.help",
			Self::Custom(name) => name,
		}
	}
}

/// Modifier key flag for a UIKit `UIKeyCommand`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyModifier {
	/// Command (⌘).
	Command,
	/// Shift (⇧).
	Shift,
	/// Option / Alt (⌥).
	Option,
	/// Control (⌃).
	Control,
}

/// A hardware keyboard shortcut registered via `UIKeyCommand` and shown in the
/// iPadOS/iOS Cmd-hold HUD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCommand {
	id: String,
	title: String,
	input: String,
	modifiers: Vec<KeyModifier>,
	discoverability_title: Option<String>,
	enabled: bool,
}

impl KeyCommand {
	/// Validates and constructs a `UIKeyCommand` descriptor.
	///
	/// `input` must be a short non-control ASCII key (such as `"k"`, `"n"`,
	/// `"["`, or `"UIKeyInputEscape"`).
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidDefinition`] for invalid IDs/titles or
	/// [`Error::InvalidAccelerator`] for invalid key inputs.
	pub fn new(
		id: impl Into<String>,
		title: impl Into<String>,
		input: impl Into<String>,
		modifiers: impl IntoIterator<Item = KeyModifier>,
	) -> Result<Self, Error> {
		let id = id.into();
		let title = title.into();
		let input = input.into();
		if !valid_id(&id) || !valid_label(&title) {
			return Err(Error::InvalidDefinition);
		}
		if input.is_empty()
			|| input.len() > 32
			|| !input.is_ascii()
			|| input.chars().any(char::is_control)
		{
			return Err(Error::InvalidAccelerator);
		}
		let mut deduped = Vec::new();
		for modifier in modifiers {
			if !deduped.contains(&modifier) {
				deduped.push(modifier);
			}
		}
		Ok(Self {
			id,
			title,
			input,
			modifiers: deduped,
			discoverability_title: None,
			enabled: true,
		})
	}

	/// Sets the optional subtitle shown in the iPadOS keyboard-shortcut HUD.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidDefinition`] if the subtitle is blank or has
	/// control characters.
	pub fn discoverability_title(mut self, subtitle: impl Into<String>) -> Result<Self, Error> {
		let subtitle = subtitle.into();
		if !valid_label(&subtitle) {
			return Err(Error::InvalidDefinition);
		}
		self.discoverability_title = Some(subtitle);
		Ok(self)
	}

	/// Enables or disables the key command.
	#[must_use]
	pub const fn enabled(mut self, enabled: bool) -> Self {
		self.enabled = enabled;
		self
	}

	/// Stable action identifier.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Key input token.
	#[must_use]
	pub fn input(&self) -> &str {
		&self.input
	}

	/// Whether the command is enabled.
	#[must_use]
	pub const fn is_enabled(&self) -> bool {
		self.enabled
	}

	/// Active modifiers.
	#[must_use]
	pub fn modifiers(&self) -> &[KeyModifier] {
		&self.modifiers
	}

	/// Optional HUD subtitle.
	#[must_use]
	pub fn subtitle(&self) -> Option<&str> {
		self.discoverability_title.as_deref()
	}

	/// Visible menu/HUD title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}
}

/// One section inserted into a UIKit menu bar slot
/// (`UIMenu.Options.displayInline`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IosMenuSection {
	id: String,
	title: String,
	placement: IosMenuPlacement,
	inline: bool,
	definition: Definition,
}

impl IosMenuSection {
	/// Validates and builds one iOS / iPadOS menu bar section.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidDefinition`] or [`Error::InvalidAccelerator`]
	/// if validation fails.
	pub fn new(
		id: impl Into<String>,
		title: impl Into<String>,
		placement: IosMenuPlacement,
		items: Vec<Item>,
	) -> Result<Self, Error> {
		let id = id.into();
		let title = title.into();
		if !valid_id(&id) || !valid_label(&title) {
			return Err(Error::InvalidDefinition);
		}
		if let IosMenuPlacement::Custom(name) = &placement
			&& !valid_id(name)
		{
			return Err(Error::InvalidDefinition);
		}
		let definition = Definition::new(items)?;
		Ok(Self {
			id,
			title,
			placement,
			inline: true,
			definition,
		})
	}

	/// Controls `UIMenu.Options.displayInline` (defaults to `true`).
	#[must_use]
	pub const fn inline(mut self, inline: bool) -> Self {
		self.inline = inline;
		self
	}

	/// Validated items in this section.
	#[must_use]
	pub fn definition(&self) -> &Definition {
		&self.definition
	}

	/// Stable section identifier.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Whether the section renders inline inside the parent menu.
	#[must_use]
	pub const fn is_inline(&self) -> bool {
		self.inline
	}

	/// Target UIKit menu bar placement.
	#[must_use]
	pub fn placement(&self) -> &IosMenuPlacement {
		&self.placement
	}

	/// Visible section title.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}
}

/// Complete validated iOS / iPadOS menu bar and hardware shortcut spec.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IosMenuBar {
	sections: Vec<IosMenuSection>,
	key_commands: Vec<KeyCommand>,
}

impl IosMenuBar {
	/// Starts an empty iOS / iPadOS menu bar builder.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Appends a validated menu section.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidDefinition`] on duplicate section IDs or when
	/// exceeding [`MAX_ITEMS`].
	pub fn section(mut self, section: IosMenuSection) -> Result<Self, Error> {
		if self.sections.len() >= MAX_ITEMS
			|| self
				.sections
				.iter()
				.any(|existing| existing.id == section.id)
		{
			return Err(Error::InvalidDefinition);
		}
		self.sections.push(section);
		Ok(self)
	}

	/// Appends a validated hardware keyboard shortcut (`UIKeyCommand`).
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidDefinition`] on duplicate IDs or
	/// [`Error::InvalidAccelerator`] on duplicate key+modifier chords.
	pub fn key_command(mut self, command: KeyCommand) -> Result<Self, Error> {
		if self.key_commands.len() >= MAX_ITEMS
			|| self
				.key_commands
				.iter()
				.any(|existing| existing.id == command.id)
		{
			return Err(Error::InvalidDefinition);
		}

		let chord = chord_key(&command);
		let mut seen = HashSet::new();
		for existing in &self.key_commands {
			seen.insert(chord_key(existing));
		}
		if !seen.insert(chord) {
			return Err(Error::InvalidAccelerator);
		}

		self.key_commands.push(command);
		Ok(self)
	}

	/// Registered menu sections.
	#[must_use]
	pub fn sections(&self) -> &[IosMenuSection] {
		&self.sections
	}

	/// Registered hardware keyboard commands.
	#[must_use]
	pub fn key_commands(&self) -> &[KeyCommand] {
		&self.key_commands
	}
}

/// Host contract for installing an [`IosMenuBar`] through UIKit's
/// `UIMenuBuilder` and `UIKeyCommand` responders.
pub trait IosMenuHost {
	/// Installs or replaces the active iOS / iPadOS menu bar.
	///
	/// # Errors
	///
	/// Returns [`Error::Unavailable`] outside iOS/iPadOS or [`Error::Native`]
	/// if UIKit rejects the rebuild.
	fn install(&mut self, menu_bar: IosMenuBar) -> Result<(), Error>;

	/// Clears previously installed custom sections and key commands.
	fn clear(&mut self) -> Result<(), Error>;
}

/// In-memory [`IosMenuHost`] for portable tests and host simulation.
#[derive(Debug, Clone, Default)]
pub struct MemoryIosMenuHost {
	active: Option<IosMenuBar>,
}

impl MemoryIosMenuHost {
	/// Creates an empty in-memory iOS menu host.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Currently installed iOS menu bar, if any.
	#[must_use]
	pub fn active(&self) -> Option<&IosMenuBar> {
		self.active.as_ref()
	}
}

impl IosMenuHost for MemoryIosMenuHost {
	fn install(&mut self, menu_bar: IosMenuBar) -> Result<(), Error> {
		self.active = Some(menu_bar);
		Ok(())
	}

	fn clear(&mut self) -> Result<(), Error> {
		self.active = None;
		Ok(())
	}
}

fn valid_id(id: &str) -> bool {
	!id.is_empty()
		&& id.len() <= 64
		&& id
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

fn chord_key(command: &KeyCommand) -> (String, u8) {
	let mut bits = 0u8;
	for modifier in &command.modifiers {
		bits |= match modifier {
			KeyModifier::Command => 1 << 0,
			KeyModifier::Shift => 1 << 1,
			KeyModifier::Option => 1 << 2,
			KeyModifier::Control => 1 << 3,
		};
	}
	(command.input.to_ascii_lowercase(), bits)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Command;

	#[test]
	fn ios_menu_bar_builds_sections_and_rejects_duplicate_chords() {
		let file_section = IosMenuSection::new(
			"file.documents",
			"Documents",
			IosMenuPlacement::File,
			vec![
				Command("new_note")
					.label("New Note")
					.accelerator("Cmd+N")
					.into(),
			],
		)
		.unwrap();

		let quick_open = KeyCommand::new("quick_open", "Quick Open", "k", [KeyModifier::Command])
			.unwrap()
			.discoverability_title("Jump to any document")
			.unwrap();

		let bar = IosMenuBar::new()
			.section(file_section)
			.unwrap()
			.key_command(quick_open)
			.unwrap();

		assert_eq!(bar.sections().len(), 1);
		assert_eq!(
			bar.sections()[0].placement().identifier(),
			"com.apple.menu.file"
		);
		assert_eq!(bar.key_commands().len(), 1);

		// Duplicate Cmd+K chord is rejected.
		let duplicate = KeyCommand::new("search", "Search", "K", [KeyModifier::Command]).unwrap();
		assert_eq!(
			bar.clone().key_command(duplicate),
			Err(Error::InvalidAccelerator)
		);

		let mut host = MemoryIosMenuHost::new();
		host.install(bar).unwrap();
		assert!(host.active().is_some());
		host.clear().unwrap();
		assert!(host.active().is_none());
	}
}
