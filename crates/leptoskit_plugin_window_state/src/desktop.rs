//! Electron-class desktop window options, always-on-top overlay levels,
//! custom chrome / vibrancy materials, background-only & tray-only lifecycle,
//! and taskbar / dock integration.
//!
//! All types in this module are portable and validated so desktop hosts on
//! macOS, Windows, and Linux — and portable unit tests — share one consistent
//! contract.

use serde::Deserialize;
use serde::Serialize;

use crate::Error;

/// Window stacking level relative to other applications.
///
/// Maps to `NSWindow.Level` on macOS, `HWND_TOPMOST` / `HWND_NOTOPMOST` on
/// Windows, and window-manager layer hints on Linux.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowLevel {
	/// Ordinary application window.
	#[default]
	Normal,
	/// Floating utility palette above normal windows (`NSFloatingWindowLevel`).
	Floating,
	/// Always drawn above other regular applications (`HWND_TOPMOST` /
	/// `NSFloatingWindowLevel`).
	AlwaysOnTop,
	/// Status-bar / menu-bar-popover level (`NSStatusWindowLevel`).
	Status,
	/// Pop-up menu level (`NSPopUpMenuWindowLevel`).
	PopUpMenu,
	/// Highest overlay level above full-screen spaces
	/// (`NSScreenSaverWindowLevel`).
	ScreenSaver,
}

impl WindowLevel {
	/// Whether this level floats above standard application windows.
	#[must_use]
	pub const fn is_always_on_top(self) -> bool {
		!matches!(self, Self::Normal)
	}
}

/// Window frame / titlebar style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowChrome {
	/// Native OS titlebar and window frame.
	#[default]
	Standard,
	/// No OS titlebar or borders (useful for HUDs, launchers, and floating
	/// overlays).
	Frameless,
	/// Native window controls kept while letting app content draw into the
	/// titlebar area (macOS full-size content view / Windows custom caption).
	HiddenTitleBar {
		/// Optional horizontal inset for macOS traffic-light buttons in logical
		/// points.
		traffic_light_x: Option<f64>,
		/// Optional vertical inset for macOS traffic-light buttons in logical
		/// points.
		traffic_light_y: Option<f64>,
	},
}

/// Platform translucent backdrop material.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VibrancyMaterial {
	/// Solid opaque window surface.
	#[default]
	None,
	/// macOS sidebar vibrancy (`NSVisualEffectMaterial.sidebar`).
	Sidebar,
	/// macOS floating HUD material (`NSVisualEffectMaterial.hudWindow`).
	HudWindow,
	/// macOS popover material (`NSVisualEffectMaterial.popover`).
	Popover,
	/// macOS under-window material
	/// (`NSVisualEffectMaterial.underWindowBackground`).
	UnderWindowBackground,
	/// Windows 11 Mica backdrop (`DWMSBT_MAINWINDOW`).
	Mica,
	/// Windows 11 Mica Alt / Tabbed backdrop (`DWMSBT_TABBEDWINDOW`).
	MicaAlt,
	/// Windows Acrylic translucent blur (`DWMSBT_TRANSIENTWINDOW`).
	Acrylic,
	/// Tabbed window material.
	Tabbed,
}

/// What happens when the user closes the last visible desktop window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloseBehavior {
	/// Terminate the process when the last window closes.
	#[default]
	Quit,
	/// Hide the window and keep the app alive in the system tray / menu bar.
	HideToTray,
	/// Keep the process running in the background with no visible windows or
	/// tray prompt required.
	KeepRunningInBackground,
	/// Minimize the window instead of closing it.
	Minimize,
}

/// How the desktop application appears in the OS launcher / Dock / taskbar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationPolicy {
	/// Standard foreground app with Dock / taskbar icon and application menu.
	#[default]
	Regular,
	/// Menu-bar / tray accessory app (`NSApplicationActivationPolicy.accessory`
	/// / `LSUIElement`, hidden from Dock/Alt-Tab until a window is shown).
	Accessory,
	/// Background-only agent with no Dock icon or menu bar until explicitly
	/// elevated.
	BackgroundOnly,
}

/// User-attention request urgency for the Dock or taskbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DockAttention {
	/// Bounce once (macOS) or flash subtly (Windows / Linux urgency hint).
	Informational,
	/// Bounce repeatedly until the user activates the app.
	Critical,
}

/// Progress indicator displayed on the Dock tile, Windows taskbar button, or
/// Linux Unity/KDE launcher entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskbarProgress {
	/// No progress indicator shown.
	#[default]
	None,
	/// Animated indeterminate spinner / marquee.
	Indeterminate,
	/// Normal determinate progress in `0.0..=1.0`.
	Normal(f64),
	/// Paused determinate progress in `0.0..=1.0`.
	Paused(f64),
	/// Error determinate progress in `0.0..=1.0`.
	Error(f64),
}

impl TaskbarProgress {
	/// Validates that any determinate fraction is finite and in `0.0..=1.0`.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidGeometry`] when a fraction is `NaN`, infinite,
	/// negative, or greater than `1.0`.
	pub fn validate(self) -> Result<Self, Error> {
		match self {
			Self::None | Self::Indeterminate => Ok(self),
			Self::Normal(value) | Self::Paused(value) | Self::Error(value) => {
				if value.is_finite() && (0.0..=1.0).contains(&value) {
					Ok(self)
				} else {
					Err(Error::InvalidGeometry)
				}
			}
		}
	}
}

/// Declarative options for constructing or updating a desktop window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct WindowOptions {
	/// Window title shown in the titlebar and OS window switcher.
	pub title: String,
	/// Initial width in logical screen units.
	pub width: f64,
	/// Initial height in logical screen units.
	pub height: f64,
	/// Optional minimum width in logical units.
	pub min_width: Option<f64>,
	/// Optional minimum height in logical units.
	pub min_height: Option<f64>,
	/// Optional maximum width in logical units.
	pub max_width: Option<f64>,
	/// Optional maximum height in logical units.
	pub max_height: Option<f64>,
	/// Stacking level (set to [`WindowLevel::AlwaysOnTop`] or
	/// [`WindowLevel::Floating`] to keep the app drawn over other apps).
	pub level: WindowLevel,
	/// Window frame / titlebar style.
	pub chrome: WindowChrome,
	/// Transparent window surface (pairs with frameless or overlay windows).
	pub transparent: bool,
	/// Native macOS vibrancy or Windows Mica/Acrylic material.
	pub vibrancy: VibrancyMaterial,
	/// Click-through mode (`ignoresMouseEvents`): pointer input passes through
	/// the window to apps underneath.
	pub ignore_mouse_events: bool,
	/// Pin the window across every virtual desktop / macOS Space.
	pub visible_on_all_workspaces: bool,
	/// Omit the window from the OS taskbar / window list.
	pub skip_taskbar: bool,
	/// Allow user resizing.
	pub resizable: bool,
	/// Allow minimizing.
	pub minimizable: bool,
	/// Allow maximizing / zooming.
	pub maximizable: bool,
	/// Allow closing via window chrome.
	pub closable: bool,
	/// Whether the window can take keyboard focus (`false` for passive HUDs).
	pub focusable: bool,
	/// Behavior when the window is closed.
	pub close_behavior: CloseBehavior,
}

impl Default for WindowOptions {
	fn default() -> Self {
		Self {
			title: String::new(),
			width: 800.0,
			height: 600.0,
			min_width: None,
			min_height: None,
			max_width: None,
			max_height: None,
			level: WindowLevel::Normal,
			chrome: WindowChrome::Standard,
			transparent: false,
			vibrancy: VibrancyMaterial::None,
			ignore_mouse_events: false,
			visible_on_all_workspaces: false,
			skip_taskbar: false,
			resizable: true,
			minimizable: true,
			maximizable: true,
			closable: true,
			focusable: true,
			close_behavior: CloseBehavior::Quit,
		}
	}
}

impl WindowOptions {
	/// Starts window options with sensible desktop defaults (`800x600`,
	/// standard chrome, resizable).
	#[must_use]
	pub fn new(title: impl Into<String>) -> Self {
		Self {
			title: title.into(),
			..Self::default()
		}
	}

	/// Preset for a floating always-on-top overlay HUD (frameless, transparent,
	/// pinned across workspaces, hidden from taskbar).
	#[must_use]
	pub fn floating_overlay(title: impl Into<String>, width: f64, height: f64) -> Self {
		Self::new(title)
			.size(width, height)
			.level(WindowLevel::AlwaysOnTop)
			.chrome(WindowChrome::Frameless)
			.transparent(true)
			.visible_on_all_workspaces(true)
			.skip_taskbar(true)
	}

	/// Keeps the window above standard apps when `enabled` is `true`.
	#[must_use]
	pub const fn always_on_top(mut self, enabled: bool) -> Self {
		self.level = if enabled {
			WindowLevel::AlwaysOnTop
		} else {
			WindowLevel::Normal
		};
		self
	}

	/// Sets the window chrome style.
	#[must_use]
	pub const fn chrome(mut self, chrome: WindowChrome) -> Self {
		self.chrome = chrome;
		self
	}

	/// Sets whether the window can be closed.
	#[must_use]
	pub const fn closable(mut self, closable: bool) -> Self {
		self.closable = closable;
		self
	}

	/// Sets what happens when the window closes (`Quit`, `HideToTray`,
	/// `KeepRunningInBackground`, or `Minimize`).
	#[must_use]
	pub const fn close_behavior(mut self, behavior: CloseBehavior) -> Self {
		self.close_behavior = behavior;
		self
	}

	/// Sets whether the window can receive keyboard focus.
	#[must_use]
	pub const fn focusable(mut self, focusable: bool) -> Self {
		self.focusable = focusable;
		self
	}

	/// Enables click-through so mouse events pass through to windows below.
	#[must_use]
	pub const fn ignore_mouse_events(mut self, ignore: bool) -> Self {
		self.ignore_mouse_events = ignore;
		self
	}

	/// Sets the exact window stacking level.
	#[must_use]
	pub const fn level(mut self, level: WindowLevel) -> Self {
		self.level = level;
		self
	}

	/// Sets maximum logical dimensions.
	#[must_use]
	pub const fn max_size(mut self, width: f64, height: f64) -> Self {
		self.max_width = Some(width);
		self.max_height = Some(height);
		self
	}

	/// Sets whether the window can be maximized.
	#[must_use]
	pub const fn maximizable(mut self, maximizable: bool) -> Self {
		self.maximizable = maximizable;
		self
	}

	/// Sets minimum logical dimensions.
	#[must_use]
	pub const fn min_size(mut self, width: f64, height: f64) -> Self {
		self.min_width = Some(width);
		self.min_height = Some(height);
		self
	}

	/// Sets whether the window can be minimized.
	#[must_use]
	pub const fn minimizable(mut self, minimizable: bool) -> Self {
		self.minimizable = minimizable;
		self
	}

	/// Sets whether the user can resize the window.
	#[must_use]
	pub const fn resizable(mut self, resizable: bool) -> Self {
		self.resizable = resizable;
		self
	}

	/// Sets initial logical dimensions.
	#[must_use]
	pub const fn size(mut self, width: f64, height: f64) -> Self {
		self.width = width;
		self.height = height;
		self
	}

	/// Hides the window from the OS taskbar / window switcher.
	#[must_use]
	pub const fn skip_taskbar(mut self, skip: bool) -> Self {
		self.skip_taskbar = skip;
		self
	}

	/// Enables a transparent window surface.
	#[must_use]
	pub const fn transparent(mut self, transparent: bool) -> Self {
		self.transparent = transparent;
		self
	}

	/// Validates finite positive sizes and consistent min/max bounds.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidGeometry`] when any dimension is nonfinite,
	/// nonpositive, or `min > max`.
	pub fn validate(&self) -> Result<(), Error> {
		let valid_positive = |value: f64| value.is_finite() && value > 0.0;
		if !valid_positive(self.width) || !valid_positive(self.height) {
			return Err(Error::InvalidGeometry);
		}
		for optional in [
			self.min_width,
			self.min_height,
			self.max_width,
			self.max_height,
		] {
			if let Some(value) = optional
				&& !valid_positive(value)
			{
				return Err(Error::InvalidGeometry);
			}
		}
		if let (Some(min_w), Some(max_w)) = (self.min_width, self.max_width)
			&& min_w > max_w
		{
			return Err(Error::InvalidGeometry);
		}
		if let (Some(min_h), Some(max_h)) = (self.min_height, self.max_height)
			&& min_h > max_h
		{
			return Err(Error::InvalidGeometry);
		}
		Ok(())
	}

	/// Sets a native vibrancy / Mica / Acrylic backdrop material.
	#[must_use]
	pub const fn vibrancy(mut self, material: VibrancyMaterial) -> Self {
		self.vibrancy = material;
		self
	}

	/// Pins the window across all virtual desktops / macOS Spaces.
	#[must_use]
	pub const fn visible_on_all_workspaces(mut self, pinned: bool) -> Self {
		self.visible_on_all_workspaces = pinned;
		self
	}
}

/// Runtime control contract implemented by desktop hosts for Electron-style
/// window and app-lifecycle manipulation.
pub trait WindowController {
	/// Applies a validated [`WindowOptions`] configuration.
	///
	/// # Errors
	///
	/// Returns [`Error::InvalidGeometry`] if `options.validate()` fails or
	/// [`Error::Unavailable`] if the host rejects a requested option.
	fn apply_options(&mut self, options: WindowOptions) -> Result<(), Error>;

	/// Shows the window.
	fn show(&mut self) -> Result<(), Error>;

	/// Hides the window without terminating the process.
	fn hide(&mut self) -> Result<(), Error>;

	/// Brings the window to the foreground and requests keyboard focus.
	fn focus(&mut self) -> Result<(), Error>;

	/// Sets the window stacking level (e.g. always-on-top overlay).
	fn set_level(&mut self, level: WindowLevel) -> Result<(), Error>;

	/// Convenience toggle between [`WindowLevel::AlwaysOnTop`] and
	/// [`WindowLevel::Normal`].
	fn set_always_on_top(&mut self, enabled: bool) -> Result<(), Error> {
		self.set_level(if enabled {
			WindowLevel::AlwaysOnTop
		} else {
			WindowLevel::Normal
		})
	}

	/// Enables or disables click-through mouse transparency.
	fn set_ignore_mouse_events(&mut self, ignore: bool) -> Result<(), Error>;

	/// Pins or unpins the window across all virtual desktops / Spaces.
	fn set_visible_on_all_workspaces(&mut self, pinned: bool) -> Result<(), Error>;

	/// Updates the OS activation policy (Dock/taskbar visibility, menu-bar
	/// accessory, or background-only agent).
	fn set_activation_policy(&mut self, policy: ActivationPolicy) -> Result<(), Error>;

	/// Sets close behavior (quit, hide to tray, keep running in background,
	/// or minimize).
	fn set_close_behavior(&mut self, behavior: CloseBehavior) -> Result<(), Error>;

	/// Updates the Dock / taskbar progress indicator.
	fn set_taskbar_progress(&mut self, progress: TaskbarProgress) -> Result<(), Error>;

	/// Requests user attention (Dock bounce / taskbar flash).
	fn request_user_attention(&mut self, attention: DockAttention) -> Result<(), Error>;

	/// Handles a user close request according to the active [`CloseBehavior`],
	/// returning `true` if the host should terminate the app or `false` if the
	/// app remains alive in the tray or background.
	fn handle_close_request(&mut self) -> Result<bool, Error>;

	/// Whether the window is currently visible.
	fn is_visible(&self) -> bool;

	/// Current stacking level.
	fn level(&self) -> WindowLevel;
}

/// In-memory [`WindowController`] for portable tests and host state tracking.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryWindowController {
	options: WindowOptions,
	visible: bool,
	focused: bool,
	minimized: bool,
	activation_policy: ActivationPolicy,
	taskbar_progress: TaskbarProgress,
	attention_requests: Vec<DockAttention>,
}

impl Default for MemoryWindowController {
	fn default() -> Self {
		Self::new(WindowOptions::default())
	}
}

impl MemoryWindowController {
	/// Creates a controller initialized with `options`.
	#[must_use]
	pub fn new(options: WindowOptions) -> Self {
		Self {
			options,
			visible: true,
			focused: false,
			minimized: false,
			activation_policy: ActivationPolicy::Regular,
			taskbar_progress: TaskbarProgress::None,
			attention_requests: Vec::new(),
		}
	}

	/// Current activation policy.
	#[must_use]
	pub const fn activation_policy(&self) -> ActivationPolicy {
		self.activation_policy
	}

	/// Recorded user-attention requests in chronological order.
	#[must_use]
	pub fn attention_requests(&self) -> &[DockAttention] {
		&self.attention_requests
	}

	/// Whether the window is currently focused.
	#[must_use]
	pub const fn is_focused(&self) -> bool {
		self.focused
	}

	/// Whether the window is currently minimized.
	#[must_use]
	pub const fn is_minimized(&self) -> bool {
		self.minimized
	}

	/// Current window options snapshot.
	#[must_use]
	pub const fn options(&self) -> &WindowOptions {
		&self.options
	}

	/// Current taskbar progress state.
	#[must_use]
	pub const fn taskbar_progress(&self) -> TaskbarProgress {
		self.taskbar_progress
	}
}

impl WindowController for MemoryWindowController {
	fn apply_options(&mut self, options: WindowOptions) -> Result<(), Error> {
		options.validate()?;
		self.options = options;
		Ok(())
	}

	fn show(&mut self) -> Result<(), Error> {
		self.visible = true;
		self.minimized = false;
		Ok(())
	}

	fn hide(&mut self) -> Result<(), Error> {
		self.visible = false;
		self.focused = false;
		Ok(())
	}

	fn focus(&mut self) -> Result<(), Error> {
		if !self.options.focusable {
			return Err(Error::Unavailable);
		}
		self.visible = true;
		self.minimized = false;
		self.focused = true;
		Ok(())
	}

	fn set_level(&mut self, level: WindowLevel) -> Result<(), Error> {
		self.options.level = level;
		Ok(())
	}

	fn set_ignore_mouse_events(&mut self, ignore: bool) -> Result<(), Error> {
		self.options.ignore_mouse_events = ignore;
		Ok(())
	}

	fn set_visible_on_all_workspaces(&mut self, pinned: bool) -> Result<(), Error> {
		self.options.visible_on_all_workspaces = pinned;
		Ok(())
	}

	fn set_activation_policy(&mut self, policy: ActivationPolicy) -> Result<(), Error> {
		self.activation_policy = policy;
		Ok(())
	}

	fn set_close_behavior(&mut self, behavior: CloseBehavior) -> Result<(), Error> {
		self.options.close_behavior = behavior;
		Ok(())
	}

	fn set_taskbar_progress(&mut self, progress: TaskbarProgress) -> Result<(), Error> {
		self.taskbar_progress = progress.validate()?;
		Ok(())
	}

	fn request_user_attention(&mut self, attention: DockAttention) -> Result<(), Error> {
		self.attention_requests.push(attention);
		Ok(())
	}

	fn handle_close_request(&mut self) -> Result<bool, Error> {
		match self.options.close_behavior {
			CloseBehavior::Quit => {
				self.visible = false;
				self.focused = false;
				Ok(true)
			}
			CloseBehavior::HideToTray | CloseBehavior::KeepRunningInBackground => {
				self.visible = false;
				self.focused = false;
				Ok(false)
			}
			CloseBehavior::Minimize => {
				self.minimized = true;
				self.focused = false;
				Ok(false)
			}
		}
	}

	fn is_visible(&self) -> bool {
		self.visible
	}

	fn level(&self) -> WindowLevel {
		self.options.level
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn floating_overlay_preset_validates_and_sets_overlay_flags() {
		let overlay = WindowOptions::floating_overlay("Teleprompter", 420.0, 140.0)
			.ignore_mouse_events(true)
			.vibrancy(VibrancyMaterial::HudWindow)
			.close_behavior(CloseBehavior::HideToTray);
		overlay.validate().unwrap();
		assert!(overlay.level.is_always_on_top());
		assert_eq!(overlay.chrome, WindowChrome::Frameless);
		assert!(overlay.transparent);
		assert!(overlay.ignore_mouse_events);
		assert!(overlay.visible_on_all_workspaces);
		assert!(overlay.skip_taskbar);
	}

	#[test]
	fn window_options_reject_invalid_sizes() {
		assert_eq!(
			WindowOptions::new("Bad").size(0.0, 200.0).validate(),
			Err(Error::InvalidGeometry)
		);
		assert_eq!(
			WindowOptions::new("Bad")
				.min_size(600.0, 400.0)
				.max_size(400.0, 500.0)
				.validate(),
			Err(Error::InvalidGeometry)
		);
		assert_eq!(
			TaskbarProgress::Normal(1.5).validate(),
			Err(Error::InvalidGeometry)
		);
		assert_eq!(
			TaskbarProgress::Normal(f64::NAN).validate(),
			Err(Error::InvalidGeometry)
		);
	}

	#[test]
	fn memory_controller_tracks_background_and_overlay_lifecycle() {
		let mut controller = MemoryWindowController::new(
			WindowOptions::new("Agent")
				.always_on_top(true)
				.close_behavior(CloseBehavior::HideToTray),
		);
		controller
			.set_activation_policy(ActivationPolicy::Accessory)
			.unwrap();
		assert_eq!(controller.activation_policy(), ActivationPolicy::Accessory);
		assert!(controller.level().is_always_on_top());

		// Closing hides to tray without terminating.
		let should_quit = controller.handle_close_request().unwrap();
		assert!(!should_quit);
		assert!(!controller.is_visible());

		// Switching to Quit terminates on close.
		controller.show().unwrap();
		controller.set_close_behavior(CloseBehavior::Quit).unwrap();
		assert!(controller.handle_close_request().unwrap());

		// Taskbar progress & user attention.
		controller
			.set_taskbar_progress(TaskbarProgress::Normal(0.65))
			.unwrap();
		assert_eq!(controller.taskbar_progress(), TaskbarProgress::Normal(0.65));
		controller
			.request_user_attention(DockAttention::Critical)
			.unwrap();
		assert_eq!(controller.attention_requests(), &[DockAttention::Critical]);
	}
}
