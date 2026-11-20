//! Runtime and compile-time target platform detection, conditional branching,
//! and capability queries.
//!
//! A leptoskit screen is shared across every platform by default, and apps
//! still need clean seams to turn on platform-specific features: an iOS/iPadOS
//! menu bar, a Chrome or Firefox sidebar, a macOS/Windows/Linux system tray or
//! always-on-top overlay, or watchOS/Wear OS hardware surfaces.
//!
//! Use [`TargetPlatform::current`] to inspect the active target,
//! [`PlatformSwitch`] to choose values by platform or family, the [`on_ios`] /
//! [`on_android`] / [`on_desktop`] / [`on_extension`] helpers to render
//! optional platform-specific UI, and [`PlatformCapabilities`] to check what
//! OS-specific features a target supports.
//!
//! ```
//! use leptoskit_native::target::Capability;
//! use leptoskit_native::target::PlatformCapabilities;
//! use leptoskit_native::target::PlatformSwitch;
//! use leptoskit_native::target::TargetPlatform;
//! use leptoskit_native::target::with_platform;
//!
//! let label = with_platform(TargetPlatform::Ios, || {
//! 	PlatformSwitch::new()
//! 		.ios("iOS menu bar & Live Activities")
//! 		.android("Android Quick Settings & Intents")
//! 		.desktop("Desktop window & tray")
//! 		.default("Shared experience")
//! 		.resolve()
//! });
//! assert_eq!(label, "iOS menu bar & Live Activities");
//!
//! let caps = PlatformCapabilities::for_platform(TargetPlatform::Macos);
//! assert!(caps.is_supported(Capability::DesktopMenuBar));
//! assert!(caps.is_supported(Capability::AlwaysOnTopWindow));
//! ```

use std::cell::Cell;
use std::fmt;
use std::str::FromStr;

thread_local! {
	#[cfg_attr(target_os = "android", expect(clippy::missing_const_for_thread_local, reason = "Android fallback TLS: rust-clippy#13422"))]
	static PLATFORM_OVERRIDE: Cell<Option<TargetPlatform>> = const { Cell::new(None) };
}

/// The platform the current application build or host is targeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum TargetPlatform {
	/// Browser web application (CSR, SSR, hydrate, or islands).
	Web,
	/// Browser extension (Chrome, Edge, Brave, Opera, Arc, Vivaldi, Firefox,
	/// Safari).
	Extension,
	/// Apple iOS and iPadOS native host.
	Ios,
	/// Android phone and tablet native host.
	Android,
	/// Apple macOS AppKit host.
	Macos,
	/// Microsoft Windows Win32 host.
	Windows,
	/// Linux GTK 4 host.
	Linux,
	/// Huawei HarmonyOS ArkUI host (experimental).
	Harmonyos,
	/// Android Wear OS watch host (experimental).
	Wearos,
	/// Apple watchOS host (experimental).
	Watchos,
	/// Meta Quest / OpenXR headset host (experimental).
	Quest,
	/// Interactive terminal (TUI) host.
	Tui,
	/// Server runtime (Axum / SSR / RPC server).
	Server,
	/// Shared / platform-neutral host or test environment.
	Shared,
}

/// High-level family grouping for [`TargetPlatform`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum PlatformFamily {
	/// Browser web app or browser extension.
	Web,
	/// Phone or tablet (iOS, Android, HarmonyOS).
	Mobile,
	/// Desktop workstation (macOS, Windows, Linux).
	Desktop,
	/// Smartwatch (watchOS, Wear OS).
	Watch,
	/// Spatial / headset (Meta Quest).
	Xr,
	/// Interactive terminal.
	Terminal,
	/// Server runtime.
	Server,
	/// Platform-neutral / shared build.
	Shared,
}

/// Whether a target platform supports a given OS-level capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum CapabilitySupport {
	/// First-class support on this platform.
	Supported,
	/// Supported conditionally (for example, requires an app extension target,
	/// specific OS version, user entitlement/permission, or browser family).
	Conditional,
	/// Not available on this platform.
	Unavailable,
}

/// Platform-specific OS features that apps can query before presenting UI or
/// invoking native integrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum Capability {
	// --- Mobile & Apple / Android OS features ---
	/// iOS / iPadOS menu bar (`UIMenuBuilder`) and hardware keyboard commands.
	IosMenuBar,
	/// Home-screen widgets (Apple `WidgetKit` / Android App Widgets).
	HomeWidgets,
	/// iOS Lock Screen accessory widgets (`AccessoryCircular`,
	/// `AccessoryRectangular`, `AccessoryInline`).
	LockScreenWidgets,
	/// iOS 18+ Control Center interactive controls (`ControlWidget`).
	ControlCenterWidgets,
	/// iOS Live Activities & Dynamic Island (`ActivityKit`) / Android Live
	/// Updates.
	LiveActivities,
	/// Android Quick Settings tiles (`TileService`).
	AndroidQuickSettingsTiles,
	/// Android conversation bubbles (`BubbleMetadata`).
	AndroidBubbles,
	/// Android explicit/implicit intents (`android_intent`).
	AndroidIntents,
	/// Android Material You dynamic color palettes.
	MaterialYouDynamicColor,
	/// App Intents / Siri Shortcuts & launcher quick actions.
	AppIntents,
	/// System search indexing (Apple `CoreSpotlight` / Android `AppSearch`).
	SystemSearchIndex,
	/// Apple Handoff / Continuity (`NSUserActivity`).
	Handoff,
	/// Incoming share extension / Android `ACTION_SEND` share target.
	IncomingShareTarget,
	/// In-app purchases (`StoreKit` 2 / Google Play Billing).
	InAppPurchases,
	/// Picture-in-Picture floating window / video overlay.
	PictureInPicture,
	/// Draw-over-other-apps system overlay (`SYSTEM_ALERT_WINDOW` on Android).
	SystemOverlayWindow,
	/// Background tasks / foreground services.
	BackgroundExecution,
	/// Foldable / hinge posture detection.
	FoldablePosture,
	/// Over-the-air signed Wasm code push updates.
	CodePush,

	// --- Watch features ---
	/// Watch face complications (watchOS `WidgetKit`/`ClockKit`, Wear OS
	/// Complications).
	WatchComplications,
	/// Wear OS tiles.
	WatchTiles,
	/// watchOS Smart Stack glances.
	WatchGlances,
	/// Digital Crown / rotary bezel input.
	DigitalCrown,
	/// Always-On Display (AOD) / ambient mode lifecycle.
	AlwaysOnDisplay,
	/// Live watch workout / exercise sessions.
	WatchWorkouts,
	/// Phone-to-watch companion messaging and data sync.
	WatchCompanionSync,
	/// Watch wrist / double-tap / hardware action button gestures.
	WatchGestures,

	// --- Desktop (Electron-class) features ---
	/// Native desktop application or window menu bar.
	DesktopMenuBar,
	/// Native desktop context menus.
	DesktopContextMenu,
	/// System tray / macOS menu bar extra (`NSStatusItem`, Windows tray, Linux
	/// KSNI).
	SystemTray,
	/// Always-on-top / floating window levels and click-through overlays.
	AlwaysOnTopWindow,
	/// Frameless, transparent, or vibrancy / Mica / Acrylic window chrome.
	CustomWindowChrome,
	/// Menu-bar-only or background-only desktop lifecycle (no Dock/taskbar
	/// window required).
	BackgroundOnlyApp,
	/// Native executable auto-updater (distinct from Wasm code push).
	AutoUpdater,
	/// Global system-wide keyboard shortcuts.
	GlobalShortcuts,
	/// Single-instance process lock and second-launch activation handoff.
	SingleInstance,
	/// Launch-at-login / OS autostart registration.
	Autostart,
	/// Dock / taskbar progress bar, badge, and user-attention bounce/flash.
	TaskbarIntegration,
	/// Native OS open/save/message dialogs.
	NativeDialogs,
	/// OS power monitor (suspend/resume/lock/idle).
	PowerMonitor,

	// --- Browser extension features ---
	/// Extension side panel (`chrome.sidePanel`) or sidebar
	/// (`browser.sidebarAction`).
	ExtensionSidePanel,
	/// Extension toolbar popup surface.
	ExtensionPopup,
	/// Extension `DevTools` panel.
	ExtensionDevtoolsPanel,
	/// Chrome offscreen document (`chrome.offscreen`).
	ExtensionOffscreen,
	/// Extension omnibox address-bar integration.
	ExtensionOmnibox,
	/// Extension native messaging host (`sendNativeMessage` / `connectNative`).
	ExtensionNativeMessaging,
	/// Extension content script injection into web pages.
	ExtensionContentScripts,
}

/// Invalid platform string returned by [`TargetPlatform::from_str`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePlatformError {
	input: String,
}

impl fmt::Display for ParsePlatformError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(formatter, "unknown leptoskit platform `{}`", self.input)
	}
}

impl std::error::Error for ParsePlatformError {}

impl TargetPlatform {
	/// Every concrete platform variant in declaration order.
	pub const ALL: [Self; 14] = [
		Self::Web,
		Self::Extension,
		Self::Ios,
		Self::Android,
		Self::Macos,
		Self::Windows,
		Self::Linux,
		Self::Harmonyos,
		Self::Wearos,
		Self::Watchos,
		Self::Quest,
		Self::Tui,
		Self::Server,
		Self::Shared,
	];

	/// Canonical kebab-case identifier matching `leptoskit.toml` and
	/// `LEPTOSKIT_PLATFORM`.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Web => "web",
			Self::Extension => "extension",
			Self::Ios => "ios",
			Self::Android => "android",
			Self::Macos => "macos",
			Self::Windows => "windows",
			Self::Linux => "linux",
			Self::Harmonyos => "harmonyos",
			Self::Wearos => "wearos",
			Self::Watchos => "watchos",
			Self::Quest => "quest",
			Self::Tui => "tui",
			Self::Server => "server",
			Self::Shared => "shared",
		}
	}

	/// Returns the active target platform.
	///
	/// Resolution order:
	/// 1. Thread-local override set via [`with_platform`] or
	///    [`set_current_platform`] (used by hosts and tests).
	/// 2. Compile-time `LEPTOSKIT_PLATFORM` environment variable (set by
	///    `leptoskit_build::platforms()` or the CLI).
	/// 3. Rust target fallback (`target_os` / `target_arch = "wasm32"`).
	#[must_use]
	pub fn current() -> Self {
		if let Some(overridden) = PLATFORM_OVERRIDE.with(Cell::get) {
			return overridden;
		}

		if let Some(env_value) = option_env!("LEPTOSKIT_PLATFORM")
			&& let Ok(parsed) = env_value.parse::<Self>()
			&& parsed != Self::Shared
		{
			return parsed;
		}

		Self::compile_time_default()
	}

	/// Default platform inferred strictly from Rust `cfg` attributes when no
	/// override or `LEPTOSKIT_PLATFORM` value is active.
	#[must_use]
	pub const fn compile_time_default() -> Self {
		#[cfg(target_os = "ios")]
		{
			Self::Ios
		}
		#[cfg(target_os = "android")]
		{
			Self::Android
		}
		#[cfg(target_os = "macos")]
		{
			Self::Macos
		}
		#[cfg(target_os = "windows")]
		{
			Self::Windows
		}
		#[cfg(all(target_os = "linux", target_env = "ohos"))]
		{
			Self::Harmonyos
		}
		#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
		{
			Self::Linux
		}
		#[cfg(target_arch = "wasm32")]
		{
			Self::Web
		}
		#[cfg(not(any(
			target_os = "ios",
			target_os = "android",
			target_os = "macos",
			target_os = "windows",
			target_os = "linux",
			target_arch = "wasm32",
		)))]
		{
			Self::Shared
		}
	}

	/// Returns the capability matrix for this platform.
	#[must_use]
	pub const fn capabilities(self) -> PlatformCapabilities {
		PlatformCapabilities::for_platform(self)
	}

	/// High-level platform family.
	#[must_use]
	pub const fn family(self) -> PlatformFamily {
		match self {
			Self::Web | Self::Extension => PlatformFamily::Web,
			Self::Ios | Self::Android | Self::Harmonyos => PlatformFamily::Mobile,
			Self::Macos | Self::Windows | Self::Linux => PlatformFamily::Desktop,
			Self::Wearos | Self::Watchos => PlatformFamily::Watch,
			Self::Quest => PlatformFamily::Xr,
			Self::Tui => PlatformFamily::Terminal,
			Self::Server => PlatformFamily::Server,
			Self::Shared => PlatformFamily::Shared,
		}
	}

	/// True on Android phones/tablets.
	#[must_use]
	pub const fn is_android(self) -> bool {
		matches!(self, Self::Android)
	}

	/// True on Apple operating systems (iOS/iPadOS, macOS, watchOS).
	#[must_use]
	pub const fn is_apple(self) -> bool {
		matches!(self, Self::Ios | Self::Macos | Self::Watchos)
	}

	/// True on desktop platforms (macOS, Windows, Linux).
	#[must_use]
	pub const fn is_desktop(self) -> bool {
		matches!(self.family(), PlatformFamily::Desktop)
	}

	/// True when running as a browser extension.
	#[must_use]
	pub const fn is_extension(self) -> bool {
		matches!(self, Self::Extension)
	}

	/// True on HarmonyOS.
	#[must_use]
	pub const fn is_harmonyos(self) -> bool {
		matches!(self, Self::Harmonyos)
	}

	/// True on iOS and iPadOS.
	#[must_use]
	pub const fn is_ios(self) -> bool {
		matches!(self, Self::Ios)
	}

	/// True on Linux desktop.
	#[must_use]
	pub const fn is_linux(self) -> bool {
		matches!(self, Self::Linux)
	}

	/// True on macOS.
	#[must_use]
	pub const fn is_macos(self) -> bool {
		matches!(self, Self::Macos)
	}

	/// True on mobile phone/tablet platforms (iOS, Android, HarmonyOS).
	#[must_use]
	pub const fn is_mobile(self) -> bool {
		matches!(self.family(), PlatformFamily::Mobile)
	}

	/// True on native widget / device hosts (mobile, desktop, watch, XR, TUI).
	#[must_use]
	pub const fn is_native(self) -> bool {
		matches!(
			self,
			Self::Ios
				| Self::Android
				| Self::Macos
				| Self::Windows
				| Self::Linux
				| Self::Harmonyos
				| Self::Wearos
				| Self::Watchos
				| Self::Quest
				| Self::Tui
		)
	}

	/// True on Meta Quest / XR headsets.
	#[must_use]
	pub const fn is_quest(self) -> bool {
		matches!(self, Self::Quest)
	}

	/// True on interactive terminal targets.
	#[must_use]
	pub const fn is_tui(self) -> bool {
		matches!(self, Self::Tui)
	}

	/// True on watch platforms (watchOS, Wear OS).
	#[must_use]
	pub const fn is_watch(self) -> bool {
		matches!(self.family(), PlatformFamily::Watch)
	}

	/// True on Apple watchOS.
	#[must_use]
	pub const fn is_watchos(self) -> bool {
		matches!(self, Self::Watchos)
	}

	/// True on Android Wear OS.
	#[must_use]
	pub const fn is_wearos(self) -> bool {
		matches!(self, Self::Wearos)
	}

	/// True on web targets (website or browser extension).
	#[must_use]
	pub const fn is_web(self) -> bool {
		matches!(self, Self::Web | Self::Extension)
	}

	/// True on Windows desktop.
	#[must_use]
	pub const fn is_windows(self) -> bool {
		matches!(self, Self::Windows)
	}
}

impl fmt::Display for TargetPlatform {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.as_str())
	}
}

impl FromStr for TargetPlatform {
	type Err = ParsePlatformError;

	fn from_str(input: &str) -> Result<Self, Self::Err> {
		match input.trim().to_ascii_lowercase().as_str() {
			"web" | "browser" => Ok(Self::Web),
			"extension" | "browser-extension" => Ok(Self::Extension),
			"ios" | "ipados" => Ok(Self::Ios),
			"android" => Ok(Self::Android),
			"macos" | "darwin" => Ok(Self::Macos),
			"windows" | "win32" => Ok(Self::Windows),
			"linux" | "gtk" => Ok(Self::Linux),
			"harmonyos" | "ohos" => Ok(Self::Harmonyos),
			"wearos" | "wear-os" => Ok(Self::Wearos),
			"watchos" | "watch-os" => Ok(Self::Watchos),
			"quest" | "oculus" | "xr" => Ok(Self::Quest),
			"tui" | "terminal" => Ok(Self::Tui),
			"server" => Ok(Self::Server),
			"shared" => Ok(Self::Shared),
			_ => {
				Err(ParsePlatformError {
					input: input.to_owned(),
				})
			}
		}
	}
}

/// Sets a thread-local platform override for native hosts or tests.
///
/// Passing `None` restores default detection.
pub fn set_current_platform(platform: Option<TargetPlatform>) {
	PLATFORM_OVERRIDE.with(|slot| slot.set(platform));
}

/// Runs `f` with [`TargetPlatform::current`] temporarily returning `platform`,
/// restoring the previous value even if `f` panics.
pub fn with_platform<T>(platform: TargetPlatform, f: impl FnOnce() -> T) -> T {
	struct Guard(Option<TargetPlatform>);

	impl Drop for Guard {
		fn drop(&mut self) {
			PLATFORM_OVERRIDE.with(|slot| slot.set(self.0));
		}
	}

	let previous = PLATFORM_OVERRIDE.with(|slot| slot.replace(Some(platform)));
	let _guard = Guard(previous);
	f()
}

/// Returns the active [`TargetPlatform`]. Shorthand for
/// [`TargetPlatform::current`].
#[must_use]
pub fn current() -> TargetPlatform {
	TargetPlatform::current()
}

/// Runs `f` and returns `Some(T)` only when the active platform matches
/// `predicate`.
pub fn when<T>(predicate: impl FnOnce(TargetPlatform) -> bool, f: impl FnOnce() -> T) -> Option<T> {
	predicate(current()).then(f)
}

/// Runs `f` only on iOS / iPadOS.
pub fn on_ios<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_ios, f)
}

/// Runs `f` only on Android.
pub fn on_android<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_android, f)
}

/// Runs `f` only on mobile platforms (iOS, Android, HarmonyOS).
pub fn on_mobile<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_mobile, f)
}

/// Runs `f` only on macOS.
pub fn on_macos<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_macos, f)
}

/// Runs `f` only on Windows.
pub fn on_windows<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_windows, f)
}

/// Runs `f` only on Linux desktop.
pub fn on_linux<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_linux, f)
}

/// Runs `f` only on desktop platforms (macOS, Windows, Linux).
pub fn on_desktop<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_desktop, f)
}

/// Runs `f` only on Apple platforms (iOS, macOS, watchOS).
pub fn on_apple<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_apple, f)
}

/// Runs `f` only on web targets (website or browser extension).
pub fn on_web<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_web, f)
}

/// Runs `f` only when running as a browser extension.
pub fn on_extension<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_extension, f)
}

/// Runs `f` only on watch targets (watchOS, Wear OS).
pub fn on_watch<T>(f: impl FnOnce() -> T) -> Option<T> {
	when(TargetPlatform::is_watch, f)
}

/// Selects a value by exact platform first, then platform family (or Apple
/// grouping), then a fallback value.
#[derive(Debug, Clone)]
pub struct PlatformSwitch<T> {
	web: Option<T>,
	extension: Option<T>,
	ios: Option<T>,
	android: Option<T>,
	macos: Option<T>,
	windows: Option<T>,
	linux: Option<T>,
	harmonyos: Option<T>,
	wearos: Option<T>,
	watchos: Option<T>,
	quest: Option<T>,
	tui: Option<T>,
	apple: Option<T>,
	mobile: Option<T>,
	desktop: Option<T>,
	watch: Option<T>,
	native: Option<T>,
	default: Option<T>,
}

impl<T> Default for PlatformSwitch<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T> PlatformSwitch<T> {
	/// Starts an empty platform switch.
	#[must_use]
	pub fn new() -> Self {
		Self {
			web: None,
			extension: None,
			ios: None,
			android: None,
			macos: None,
			windows: None,
			linux: None,
			harmonyos: None,
			wearos: None,
			watchos: None,
			quest: None,
			tui: None,
			apple: None,
			mobile: None,
			desktop: None,
			watch: None,
			native: None,
			default: None,
		}
	}

	/// Value for Android.
	#[must_use]
	pub fn android(mut self, value: T) -> Self {
		self.android = Some(value);
		self
	}

	/// Value for Apple platforms (iOS, macOS, watchOS) when no exact platform
	/// branch matched.
	#[must_use]
	pub fn apple(mut self, value: T) -> Self {
		self.apple = Some(value);
		self
	}

	/// Fallback value when no specific platform or family branch matched.
	#[must_use]
	pub fn default(mut self, value: T) -> Self {
		self.default = Some(value);
		self
	}

	/// Value for desktop platforms (macOS, Windows, Linux).
	#[must_use]
	pub fn desktop(mut self, value: T) -> Self {
		self.desktop = Some(value);
		self
	}

	/// Value for browser extensions.
	#[must_use]
	pub fn extension(mut self, value: T) -> Self {
		self.extension = Some(value);
		self
	}

	/// Value for HarmonyOS.
	#[must_use]
	pub fn harmonyos(mut self, value: T) -> Self {
		self.harmonyos = Some(value);
		self
	}

	/// Value for iOS / iPadOS.
	#[must_use]
	pub fn ios(mut self, value: T) -> Self {
		self.ios = Some(value);
		self
	}

	/// Value for Linux desktop.
	#[must_use]
	pub fn linux(mut self, value: T) -> Self {
		self.linux = Some(value);
		self
	}

	/// Value for macOS.
	#[must_use]
	pub fn macos(mut self, value: T) -> Self {
		self.macos = Some(value);
		self
	}

	/// Value for mobile platforms (iOS, Android, HarmonyOS).
	#[must_use]
	pub fn mobile(mut self, value: T) -> Self {
		self.mobile = Some(value);
		self
	}

	/// Value for any native platform.
	#[must_use]
	pub fn native(mut self, value: T) -> Self {
		self.native = Some(value);
		self
	}

	/// Value for Meta Quest / XR.
	#[must_use]
	pub fn quest(mut self, value: T) -> Self {
		self.quest = Some(value);
		self
	}

	/// Resolves against [`TargetPlatform::current`], panicking if no branch and
	/// no `.default(...)` was provided. Use [`Self::try_resolve`] when a branch
	/// is optional.
	///
	/// # Panics
	///
	/// Panics if no matching branch or default value was set.
	#[must_use]
	pub fn resolve(self) -> T {
		self.resolve_for(current())
	}

	/// Resolves against an explicit `platform`, panicking if neither a matching
	/// branch nor `.default(...)` was supplied.
	///
	/// # Panics
	///
	/// Panics if no matching branch or default value was set.
	#[must_use]
	pub fn resolve_for(self, platform: TargetPlatform) -> T {
		self.try_resolve_for(platform)
			.expect("PlatformSwitch had no matching platform branch or default value")
	}

	/// Resolves against [`TargetPlatform::current`], returning `None` if no
	/// branch matched.
	#[must_use]
	pub fn try_resolve(self) -> Option<T> {
		self.try_resolve_for(current())
	}

	/// Resolves against an explicit `platform`.
	///
	/// Precedence: exact platform -> family (`mobile`/`desktop`/`watch`) ->
	/// `apple` -> `native` -> `web` (for extensions) -> `default`.
	#[must_use]
	pub fn try_resolve_for(mut self, platform: TargetPlatform) -> Option<T> {
		let exact = match platform {
			TargetPlatform::Web => self.web.take(),
			TargetPlatform::Extension => self.extension.take(),
			TargetPlatform::Ios => self.ios.take(),
			TargetPlatform::Android => self.android.take(),
			TargetPlatform::Macos => self.macos.take(),
			TargetPlatform::Windows => self.windows.take(),
			TargetPlatform::Linux => self.linux.take(),
			TargetPlatform::Harmonyos => self.harmonyos.take(),
			TargetPlatform::Wearos => self.wearos.take(),
			TargetPlatform::Watchos => self.watchos.take(),
			TargetPlatform::Quest => self.quest.take(),
			TargetPlatform::Tui => self.tui.take(),
			TargetPlatform::Server | TargetPlatform::Shared => None,
		};

		if exact.is_some() {
			return exact;
		}

		let family_match = match platform.family() {
			PlatformFamily::Mobile => self.mobile.take(),
			PlatformFamily::Desktop => self.desktop.take(),
			PlatformFamily::Watch => self.watch.take(),
			PlatformFamily::Web => {
				if platform == TargetPlatform::Extension {
					self.web.take()
				} else {
					None
				}
			}
			_ => None,
		};

		if family_match.is_some() {
			return family_match;
		}

		if platform.is_apple() && self.apple.is_some() {
			return self.apple.take();
		}

		if platform.is_native() && self.native.is_some() {
			return self.native.take();
		}

		self.default.take()
	}

	/// Value for interactive terminal (TUI).
	#[must_use]
	pub fn tui(mut self, value: T) -> Self {
		self.tui = Some(value);
		self
	}

	/// Value for watch platforms (watchOS, Wear OS).
	#[must_use]
	pub fn watch(mut self, value: T) -> Self {
		self.watch = Some(value);
		self
	}

	/// Value for watchOS.
	#[must_use]
	pub fn watchos(mut self, value: T) -> Self {
		self.watchos = Some(value);
		self
	}

	/// Value for Wear OS.
	#[must_use]
	pub fn wearos(mut self, value: T) -> Self {
		self.wearos = Some(value);
		self
	}

	/// Value for web apps (and extensions when `.extension(...)` is not set).
	#[must_use]
	pub fn web(mut self, value: T) -> Self {
		self.web = Some(value);
		self
	}

	/// Value for Windows.
	#[must_use]
	pub fn windows(mut self, value: T) -> Self {
		self.windows = Some(value);
		self
	}
}

/// Capability query table for a [`TargetPlatform`].
///
/// Reflects what the OS and leptoskit's platform APIs support on each target,
/// distinguishing first-class support (`Supported`), conditional support that
/// requires host setup / OS version / browser family (`Conditional`), and
/// unavailable features (`Unavailable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCapabilities {
	platform: TargetPlatform,
}

impl PlatformCapabilities {
	/// Returns capabilities for [`TargetPlatform::current`].
	#[must_use]
	pub fn current() -> Self {
		Self::for_platform(TargetPlatform::current())
	}

	/// Returns capabilities for `platform`.
	#[must_use]
	pub const fn for_platform(platform: TargetPlatform) -> Self {
		Self { platform }
	}

	/// True when `capability` is either [`CapabilitySupport::Supported`] or
	/// [`CapabilitySupport::Conditional`].
	#[must_use]
	pub const fn is_available(self, capability: Capability) -> bool {
		!matches!(self.support(capability), CapabilitySupport::Unavailable)
	}

	/// True only when `capability` has [`CapabilitySupport::Supported`].
	#[must_use]
	pub const fn is_supported(self, capability: Capability) -> bool {
		matches!(self.support(capability), CapabilitySupport::Supported)
	}

	/// The platform this capability view describes.
	#[must_use]
	pub const fn platform(self) -> TargetPlatform {
		self.platform
	}

	/// Looks up the support level for `capability` on this platform.
	#[must_use]
	#[allow(
		clippy::match_same_arms,
		reason = "match arms are grouped by target platform for clarity"
	)]
	pub const fn support(self, capability: Capability) -> CapabilitySupport {
		use Capability as C;
		use CapabilitySupport::Conditional;
		use CapabilitySupport::Supported;
		use CapabilitySupport::Unavailable;
		use TargetPlatform as P;

		match (self.platform, capability) {
			// --- iOS / iPadOS ---
			(
				P::Ios,
				C::IosMenuBar
				| C::HomeWidgets
				| C::LockScreenWidgets
				| C::ControlCenterWidgets
				| C::LiveActivities
				| C::AppIntents
				| C::SystemSearchIndex
				| C::Handoff
				| C::IncomingShareTarget
				| C::InAppPurchases
				| C::BackgroundExecution
				| C::CodePush,
			) => Supported,
			(P::Ios, C::PictureInPicture | C::FoldablePosture | C::DesktopContextMenu) => {
				Conditional
			}

			// --- Android ---
			(
				P::Android,
				C::HomeWidgets
				| C::AndroidQuickSettingsTiles
				| C::AndroidBubbles
				| C::AndroidIntents
				| C::MaterialYouDynamicColor
				| C::AppIntents
				| C::SystemSearchIndex
				| C::IncomingShareTarget
				| C::InAppPurchases
				| C::PictureInPicture
				| C::SystemOverlayWindow
				| C::BackgroundExecution
				| C::FoldablePosture
				| C::CodePush,
			) => Supported,
			(P::Android, C::LiveActivities) => Conditional,

			// --- watchOS ---
			(
				P::Watchos,
				C::WatchComplications
				| C::WatchGlances
				| C::DigitalCrown
				| C::AlwaysOnDisplay
				| C::WatchWorkouts
				| C::WatchCompanionSync
				| C::WatchGestures
				| C::Handoff,
			) => Supported,

			// --- Wear OS ---
			(
				P::Wearos,
				C::WatchComplications
				| C::WatchTiles
				| C::DigitalCrown
				| C::AlwaysOnDisplay
				| C::WatchWorkouts
				| C::WatchCompanionSync
				| C::WatchGestures,
			) => Supported,

			// --- macOS ---
			(
				P::Macos,
				C::DesktopMenuBar
				| C::DesktopContextMenu
				| C::SystemTray
				| C::AlwaysOnTopWindow
				| C::CustomWindowChrome
				| C::BackgroundOnlyApp
				| C::AutoUpdater
				| C::GlobalShortcuts
				| C::SingleInstance
				| C::Autostart
				| C::TaskbarIntegration
				| C::NativeDialogs
				| C::PowerMonitor
				| C::HomeWidgets
				| C::Handoff
				| C::SystemSearchIndex,
			) => Supported,
			(P::Macos, C::InAppPurchases | C::AppIntents) => Conditional,

			// --- Windows ---
			(
				P::Windows,
				C::DesktopMenuBar
				| C::DesktopContextMenu
				| C::SystemTray
				| C::AlwaysOnTopWindow
				| C::CustomWindowChrome
				| C::BackgroundOnlyApp
				| C::AutoUpdater
				| C::GlobalShortcuts
				| C::SingleInstance
				| C::Autostart
				| C::TaskbarIntegration
				| C::NativeDialogs
				| C::PowerMonitor,
			) => Supported,

			// --- Linux ---
			(
				P::Linux,
				C::DesktopContextMenu
				| C::SystemTray
				| C::AlwaysOnTopWindow
				| C::CustomWindowChrome
				| C::BackgroundOnlyApp
				| C::AutoUpdater
				| C::SingleInstance
				| C::Autostart
				| C::TaskbarIntegration
				| C::NativeDialogs
				| C::PowerMonitor,
			) => Supported,
			// Global shortcuts work on X11; Wayland needs portal support.
			// Menu bars depend on desktop environment / tray KSNI.
			(P::Linux, C::GlobalShortcuts | C::DesktopMenuBar) => Conditional,

			// --- Browser extensions ---
			(P::Extension, C::ExtensionPopup | C::ExtensionContentScripts) => Supported,
			// Side panel, DevTools, offscreen, omnibox, and native messaging
			// vary by browser family (Chromium vs Firefox vs Safari).
			(
				P::Extension,
				C::ExtensionSidePanel
				| C::ExtensionDevtoolsPanel
				| C::ExtensionOffscreen
				| C::ExtensionOmnibox
				| C::ExtensionNativeMessaging,
			) => Conditional,

			// --- Web ---
			(P::Web, C::FoldablePosture | C::PictureInPicture) => Conditional,

			// --- HarmonyOS / Quest ---
			(P::Harmonyos, C::FoldablePosture | C::HomeWidgets) => Conditional,
			(P::Quest, C::AndroidIntents | C::BackgroundExecution) => Conditional,

			_ => Unavailable,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_platform_round_trips_through_str() {
		for platform in TargetPlatform::ALL {
			assert_eq!(
				platform.as_str().parse::<TargetPlatform>().unwrap(),
				platform
			);
			assert_eq!(platform.to_string(), platform.as_str());
		}

		assert_eq!(
			"ipados".parse::<TargetPlatform>().unwrap(),
			TargetPlatform::Ios
		);
		assert_eq!(
			"darwin".parse::<TargetPlatform>().unwrap(),
			TargetPlatform::Macos
		);
		assert_eq!(
			"wear-os".parse::<TargetPlatform>().unwrap(),
			TargetPlatform::Wearos
		);
		assert!("unknown-os".parse::<TargetPlatform>().is_err());
	}

	#[test]
	fn platform_family_and_predicates_match() {
		assert!(TargetPlatform::Ios.is_mobile());
		assert!(TargetPlatform::Ios.is_apple());
		assert!(TargetPlatform::Ios.is_native());
		assert!(!TargetPlatform::Ios.is_desktop());

		assert!(TargetPlatform::Macos.is_desktop());
		assert!(TargetPlatform::Macos.is_apple());
		assert!(TargetPlatform::Windows.is_desktop());
		assert!(TargetPlatform::Linux.is_desktop());

		assert!(TargetPlatform::Watchos.is_watch());
		assert!(TargetPlatform::Watchos.is_apple());
		assert!(TargetPlatform::Wearos.is_watch());
		assert!(!TargetPlatform::Wearos.is_apple());

		assert!(TargetPlatform::Extension.is_web());
		assert!(TargetPlatform::Extension.is_extension());
		assert!(!TargetPlatform::Extension.is_native());
	}

	#[test]
	fn with_platform_scopes_and_restores_override() {
		let before = current();
		let inside = with_platform(TargetPlatform::Ios, || {
			assert_eq!(current(), TargetPlatform::Ios);
			assert!(on_ios(|| "apple").is_some());
			assert!(on_android(|| "droid").is_none());
			assert!(on_mobile(|| "mobile").is_some());
			assert!(on_desktop(|| "desk").is_none());
			"ok"
		});
		assert_eq!(inside, "ok");
		assert_eq!(current(), before);
	}

	#[test]
	fn platform_switch_precedence_picks_most_specific_branch() {
		let switch_ios = PlatformSwitch::new()
			.ios("exact-ios")
			.mobile("family-mobile")
			.apple("group-apple")
			.native("group-native")
			.default("fallback");
		assert_eq!(switch_ios.resolve_for(TargetPlatform::Ios), "exact-ios");

		let switch_android = PlatformSwitch::new()
			.ios("exact-ios")
			.mobile("family-mobile")
			.default("fallback");
		assert_eq!(
			switch_android.resolve_for(TargetPlatform::Android),
			"family-mobile"
		);

		let switch_apple = PlatformSwitch::new()
			.apple("group-apple")
			.default("fallback");
		assert_eq!(
			switch_apple.clone().resolve_for(TargetPlatform::Watchos),
			"group-apple"
		);
		assert_eq!(switch_apple.resolve_for(TargetPlatform::Wearos), "fallback");

		let switch_ext = PlatformSwitch::new().web("shared-web").default("fallback");
		assert_eq!(
			switch_ext.resolve_for(TargetPlatform::Extension),
			"shared-web"
		);

		let switch_ext_exact = PlatformSwitch::new()
			.web("shared-web")
			.extension("exact-ext")
			.default("fallback");
		assert_eq!(
			switch_ext_exact.resolve_for(TargetPlatform::Extension),
			"exact-ext"
		);
	}

	#[test]
	fn capabilities_distinguish_platform_features() {
		let ios = PlatformCapabilities::for_platform(TargetPlatform::Ios);
		assert!(ios.is_supported(Capability::IosMenuBar));
		assert!(ios.is_supported(Capability::LiveActivities));
		assert!(ios.is_supported(Capability::ControlCenterWidgets));
		assert!(!ios.is_available(Capability::AndroidQuickSettingsTiles));

		let android = PlatformCapabilities::for_platform(TargetPlatform::Android);
		assert!(android.is_supported(Capability::AndroidQuickSettingsTiles));
		assert!(android.is_supported(Capability::AndroidBubbles));
		assert!(android.is_supported(Capability::SystemOverlayWindow));
		assert_eq!(
			android.support(Capability::LiveActivities),
			CapabilitySupport::Conditional
		);
		assert!(!android.is_available(Capability::Handoff));

		let macos = PlatformCapabilities::for_platform(TargetPlatform::Macos);
		assert!(macos.is_supported(Capability::DesktopMenuBar));
		assert!(macos.is_supported(Capability::AlwaysOnTopWindow));
		assert!(macos.is_supported(Capability::BackgroundOnlyApp));
		assert!(macos.is_supported(Capability::AutoUpdater));

		let watchos = PlatformCapabilities::for_platform(TargetPlatform::Watchos);
		assert!(watchos.is_supported(Capability::DigitalCrown));
		assert!(watchos.is_supported(Capability::WatchGlances));
		assert!(!watchos.is_available(Capability::WatchTiles));

		let wearos = PlatformCapabilities::for_platform(TargetPlatform::Wearos);
		assert!(wearos.is_supported(Capability::WatchTiles));
		assert!(wearos.is_supported(Capability::AlwaysOnDisplay));
		assert!(!wearos.is_available(Capability::WatchGlances));

		let ext = PlatformCapabilities::for_platform(TargetPlatform::Extension);
		assert!(ext.is_supported(Capability::ExtensionPopup));
		assert_eq!(
			ext.support(Capability::ExtensionSidePanel),
			CapabilitySupport::Conditional
		);
	}
}
