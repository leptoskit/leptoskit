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
///
/// Covers both features backed by built-in leptoskit crates/plugins (where
/// [`Self::has_builtin_plugin`] returns `true` and [`Self::plugin_crate`]
/// names the crate) and major OS capabilities that require host-supplied
/// native code today (where [`Self::plugin_crate`] returns `None`).
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
	/// iOS Live Activities & Dynamic Island (`ActivityKit`) / Android 16+ Live
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
	/// Local and push notifications (APNs / FCM / desktop notifications).
	PushNotifications,
	/// App icon / Dock badge counts.
	IconBadge,
	/// Bluetooth Low Energy (BLE) GATT central scanning and notifications.
	BluetoothLowEnergy,
	/// Near Field Communication (Core NFC / Android NFC) NDEF tag reading.
	NfcReader,
	/// Health data stores (`HealthKit` / Android Health Connect / HarmonyOS
	/// Health Service Kit).
	HealthStore,
	/// Local biometrics (Face ID / Touch ID / `BiometricPrompt`) and
	/// hardware-backed signing keys (Secure Enclave / Android `StrongBox`).
	BiometricsAndHardwareKeys,
	/// Passkeys (`WebAuthn`) and Sign in with Apple.
	Passkeys,
	/// Augmented reality (`ARKit` / `ARCore` / `OpenXR` / `WebXR`).
	AugmentedReality,
	/// Apple Live Photos (`PHLivePhotoView`) playback.
	LivePhotos,
	/// iOS App Tracking Transparency (`ATTrackingManager`) consent prompt.
	AppTrackingTransparency,
	/// Android exact, wake-up, boot-persisted, and headless alarms
	/// (`AlarmManager`).
	AndroidExactAlarms,
	/// Android device administration (`DevicePolicyManager`).
	AndroidDeviceAdmin,
	/// Android `UsageStatsManager` screen-time/usage events and Apple
	/// lock/unlock events.
	AndroidAppUsage,
	/// Android `FLAG_SECURE` window protection and iOS capture/screenshot
	/// observation.
	ScreenCaptureProtection,
	/// Native App Store / Google Play in-app review prompt.
	StoreReviewPrompt,
	/// Multi-window scenes (iPadOS Stage Manager / Split View and Android
	/// multi-window).
	MultiWindowScenes,
	/// Apple Pencil and Android stylus pointer input and preferences.
	StylusInput,
	/// Android 14+ predictive back gesture animations.
	AndroidPredictiveBack,
	/// Lock Screen Now Playing (`MPNowPlayingInfoCenter`) and Android
	/// `MediaSession` transport controls.
	MediaSession,
	/// Live speech-to-text recognition (`SFSpeechRecognizer` / Android
	/// `SpeechRecognizer`).
	SpeechRecognition,
	/// Camera document and barcode/QR scanning (`VisionKit` / ML Kit).
	DocumentAndBarcodeScanner,
	/// Digital wallet passes (`PassKit` Apple Wallet / Google Wallet).
	WalletPasses,
	/// Native checkout sheets (Apple Pay / Google Pay).
	NativePayments,
	/// System `VoIP` calling UI (`CallKit` / `PushKit` / Android
	/// `ConnectionService`).
	VoipCalling,
	/// Background OS geofencing and region triggers (`CLMonitor` /
	/// `GeofencingClient`).
	Geofencing,
	/// External media casting (`AirPlay` / Google Cast).
	MediaCasting,
	/// Lightweight instant app experiences (Apple App Clips / Google Play
	/// Instant).
	AppClips,
	/// Apple `SharePlay` (`GroupActivities`) shared sessions.
	SharePlay,
	/// OS cloud key-value and record sync (`CloudKit` / Android Backup).
	CloudKeyValueSync,
	/// On-device AI inference (Apple Foundation Models / Core ML / Android
	/// Gemini Nano).
	OnDeviceAi,
	/// Google Play In-App Updates (`AppUpdateManager`) for native binaries.
	InAppUpdates,
	/// Hardware app attestation (Apple App Attest / Google Play Integrity).
	AppAttestation,
	/// Apple Screen Time parental and focus controls (`FamilyControls` /
	/// `ManagedSettings`).
	ScreenTimeControls,

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

impl Capability {
	/// Every defined platform capability in declaration order.
	pub const ALL: &'static [Self] = &[
		Self::IosMenuBar,
		Self::HomeWidgets,
		Self::LockScreenWidgets,
		Self::ControlCenterWidgets,
		Self::LiveActivities,
		Self::AndroidQuickSettingsTiles,
		Self::AndroidBubbles,
		Self::AndroidIntents,
		Self::MaterialYouDynamicColor,
		Self::AppIntents,
		Self::SystemSearchIndex,
		Self::Handoff,
		Self::IncomingShareTarget,
		Self::InAppPurchases,
		Self::PictureInPicture,
		Self::SystemOverlayWindow,
		Self::BackgroundExecution,
		Self::FoldablePosture,
		Self::CodePush,
		Self::PushNotifications,
		Self::IconBadge,
		Self::BluetoothLowEnergy,
		Self::NfcReader,
		Self::HealthStore,
		Self::BiometricsAndHardwareKeys,
		Self::Passkeys,
		Self::AugmentedReality,
		Self::LivePhotos,
		Self::AppTrackingTransparency,
		Self::AndroidExactAlarms,
		Self::AndroidDeviceAdmin,
		Self::AndroidAppUsage,
		Self::ScreenCaptureProtection,
		Self::StoreReviewPrompt,
		Self::MultiWindowScenes,
		Self::StylusInput,
		Self::AndroidPredictiveBack,
		Self::MediaSession,
		Self::SpeechRecognition,
		Self::DocumentAndBarcodeScanner,
		Self::WalletPasses,
		Self::NativePayments,
		Self::VoipCalling,
		Self::Geofencing,
		Self::MediaCasting,
		Self::AppClips,
		Self::SharePlay,
		Self::CloudKeyValueSync,
		Self::OnDeviceAi,
		Self::InAppUpdates,
		Self::AppAttestation,
		Self::ScreenTimeControls,
		Self::WatchComplications,
		Self::WatchTiles,
		Self::WatchGlances,
		Self::DigitalCrown,
		Self::AlwaysOnDisplay,
		Self::WatchWorkouts,
		Self::WatchCompanionSync,
		Self::WatchGestures,
		Self::DesktopMenuBar,
		Self::DesktopContextMenu,
		Self::SystemTray,
		Self::AlwaysOnTopWindow,
		Self::CustomWindowChrome,
		Self::BackgroundOnlyApp,
		Self::AutoUpdater,
		Self::GlobalShortcuts,
		Self::SingleInstance,
		Self::Autostart,
		Self::TaskbarIntegration,
		Self::NativeDialogs,
		Self::PowerMonitor,
		Self::ExtensionSidePanel,
		Self::ExtensionPopup,
		Self::ExtensionDevtoolsPanel,
		Self::ExtensionOffscreen,
		Self::ExtensionOmnibox,
		Self::ExtensionNativeMessaging,
		Self::ExtensionContentScripts,
	];

	/// Canonical kebab-case capability identifier matching `serde` output.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::IosMenuBar => "ios-menu-bar",
			Self::HomeWidgets => "home-widgets",
			Self::LockScreenWidgets => "lock-screen-widgets",
			Self::ControlCenterWidgets => "control-center-widgets",
			Self::LiveActivities => "live-activities",
			Self::AndroidQuickSettingsTiles => "android-quick-settings-tiles",
			Self::AndroidBubbles => "android-bubbles",
			Self::AndroidIntents => "android-intents",
			Self::MaterialYouDynamicColor => "material-you-dynamic-color",
			Self::AppIntents => "app-intents",
			Self::SystemSearchIndex => "system-search-index",
			Self::Handoff => "handoff",
			Self::IncomingShareTarget => "incoming-share-target",
			Self::InAppPurchases => "in-app-purchases",
			Self::PictureInPicture => "picture-in-picture",
			Self::SystemOverlayWindow => "system-overlay-window",
			Self::BackgroundExecution => "background-execution",
			Self::FoldablePosture => "foldable-posture",
			Self::CodePush => "code-push",
			Self::PushNotifications => "push-notifications",
			Self::IconBadge => "icon-badge",
			Self::BluetoothLowEnergy => "bluetooth-low-energy",
			Self::NfcReader => "nfc-reader",
			Self::HealthStore => "health-store",
			Self::BiometricsAndHardwareKeys => "biometrics-and-hardware-keys",
			Self::Passkeys => "passkeys",
			Self::AugmentedReality => "augmented-reality",
			Self::LivePhotos => "live-photos",
			Self::AppTrackingTransparency => "app-tracking-transparency",
			Self::AndroidExactAlarms => "android-exact-alarms",
			Self::AndroidDeviceAdmin => "android-device-admin",
			Self::AndroidAppUsage => "android-app-usage",
			Self::ScreenCaptureProtection => "screen-capture-protection",
			Self::StoreReviewPrompt => "store-review-prompt",
			Self::MultiWindowScenes => "multi-window-scenes",
			Self::StylusInput => "stylus-input",
			Self::AndroidPredictiveBack => "android-predictive-back",
			Self::MediaSession => "media-session",
			Self::SpeechRecognition => "speech-recognition",
			Self::DocumentAndBarcodeScanner => "document-and-barcode-scanner",
			Self::WalletPasses => "wallet-passes",
			Self::NativePayments => "native-payments",
			Self::VoipCalling => "voip-calling",
			Self::Geofencing => "geofencing",
			Self::MediaCasting => "media-casting",
			Self::AppClips => "app-clips",
			Self::SharePlay => "share-play",
			Self::CloudKeyValueSync => "cloud-key-value-sync",
			Self::OnDeviceAi => "on-device-ai",
			Self::InAppUpdates => "in-app-updates",
			Self::AppAttestation => "app-attestation",
			Self::ScreenTimeControls => "screen-time-controls",
			Self::WatchComplications => "watch-complications",
			Self::WatchTiles => "watch-tiles",
			Self::WatchGlances => "watch-glances",
			Self::DigitalCrown => "digital-crown",
			Self::AlwaysOnDisplay => "always-on-display",
			Self::WatchWorkouts => "watch-workouts",
			Self::WatchCompanionSync => "watch-companion-sync",
			Self::WatchGestures => "watch-gestures",
			Self::DesktopMenuBar => "desktop-menu-bar",
			Self::DesktopContextMenu => "desktop-context-menu",
			Self::SystemTray => "system-tray",
			Self::AlwaysOnTopWindow => "always-on-top-window",
			Self::CustomWindowChrome => "custom-window-chrome",
			Self::BackgroundOnlyApp => "background-only-app",
			Self::AutoUpdater => "auto-updater",
			Self::GlobalShortcuts => "global-shortcuts",
			Self::SingleInstance => "single-instance",
			Self::Autostart => "autostart",
			Self::TaskbarIntegration => "taskbar-integration",
			Self::NativeDialogs => "native-dialogs",
			Self::PowerMonitor => "power-monitor",
			Self::ExtensionSidePanel => "extension-side-panel",
			Self::ExtensionPopup => "extension-popup",
			Self::ExtensionDevtoolsPanel => "extension-devtools-panel",
			Self::ExtensionOffscreen => "extension-offscreen",
			Self::ExtensionOmnibox => "extension-omnibox",
			Self::ExtensionNativeMessaging => "extension-native-messaging",
			Self::ExtensionContentScripts => "extension-content-scripts",
		}
	}

	/// Optional Cargo feature on the `leptoskit` facade crate that exposes this
	/// capability's Rust types (typically under `leptoskit::platform::*`).
	#[must_use]
	pub const fn cargo_feature(self) -> Option<&'static str> {
		match self {
			Self::IosMenuBar | Self::DesktopMenuBar | Self::DesktopContextMenu => Some("menu"),
			Self::HomeWidgets
			| Self::LockScreenWidgets
			| Self::ControlCenterWidgets
			| Self::AndroidQuickSettingsTiles => Some("home-widgets"),
			Self::LiveActivities => Some("live-activities"),
			Self::AndroidIntents => Some("android-intent"),
			Self::AndroidBubbles
			| Self::MaterialYouDynamicColor
			| Self::AppIntents
			| Self::SystemSearchIndex
			| Self::Handoff
			| Self::IncomingShareTarget
			| Self::MultiWindowScenes => Some("app-shortcuts"),
			Self::CodePush => Some("codepush"),
			Self::WatchComplications
			| Self::WatchTiles
			| Self::WatchGlances
			| Self::DigitalCrown
			| Self::AlwaysOnDisplay
			| Self::WatchGestures => Some("watch"),
			Self::AlwaysOnTopWindow
			| Self::CustomWindowChrome
			| Self::BackgroundOnlyApp
			| Self::TaskbarIntegration => Some("window-state"),
			Self::AutoUpdater => Some("updater"),
			Self::ExtensionSidePanel
			| Self::ExtensionPopup
			| Self::ExtensionDevtoolsPanel
			| Self::ExtensionOffscreen
			| Self::ExtensionOmnibox
			| Self::ExtensionNativeMessaging
			| Self::ExtensionContentScripts => Some("extension"),
			_ => None,
		}
	}

	/// True when leptoskit ships a built-in crate or runtime plugin for this
	/// capability (as opposed to an OS feature that requires host-supplied
	/// native code today).
	#[must_use]
	pub const fn has_builtin_plugin(self) -> bool {
		self.plugin_crate().is_some()
	}

	/// The built-in leptoskit crate that exposes this capability, or `None`
	/// when no built-in native plugin is wired yet.
	#[must_use]
	pub const fn plugin_crate(self) -> Option<&'static str> {
		match self {
			Self::IosMenuBar | Self::DesktopMenuBar | Self::DesktopContextMenu => {
				Some("leptoskit_plugin_menu")
			}
			Self::HomeWidgets
			| Self::LockScreenWidgets
			| Self::ControlCenterWidgets
			| Self::AndroidQuickSettingsTiles => Some("leptoskit_plugin_home_widgets"),
			Self::LiveActivities => Some("leptoskit_plugin_live_activities"),
			Self::AndroidIntents => Some("leptoskit_plugin_android_intent"),
			Self::AndroidBubbles
			| Self::MaterialYouDynamicColor
			| Self::AppIntents
			| Self::SystemSearchIndex
			| Self::Handoff
			| Self::IncomingShareTarget
			| Self::MultiWindowScenes => Some("leptoskit_plugin_app_shortcuts"),
			Self::PictureInPicture | Self::SystemOverlayWindow => {
				Some("leptoskit_plugin_picture_in_picture")
			}
			Self::BackgroundExecution => Some("leptoskit_plugin_background"),
			Self::FoldablePosture => Some("leptoskit_plugin_posture"),
			Self::CodePush => Some("leptoskit_codepush"),
			Self::PushNotifications => Some("leptoskit_plugin_notifications"),
			Self::IconBadge => Some("leptoskit_plugin_badge"),
			Self::BluetoothLowEnergy => Some("leptoskit_plugin_ble"),
			Self::NfcReader => Some("leptoskit_plugin_nfc"),
			Self::HealthStore => Some("leptoskit_plugin_health"),
			Self::BiometricsAndHardwareKeys => Some("leptoskit_plugin_local_authentication"),
			Self::Passkeys => Some("leptoskit_plugin_passkeys"),
			Self::AugmentedReality => Some("leptoskit_xr"),
			Self::LivePhotos => Some("leptoskit_plugin_live_photo"),
			Self::AppTrackingTransparency => Some("leptoskit_plugin_tracking_transparency"),
			Self::AndroidExactAlarms => Some("leptoskit_plugin_android_alarm_manager"),
			Self::AndroidDeviceAdmin => Some("leptoskit_plugin_device_policy"),
			Self::AndroidAppUsage => Some("leptoskit_plugin_app_usage"),
			Self::ScreenCaptureProtection => Some("leptoskit_plugin_screen_capture"),
			Self::StoreReviewPrompt => Some("leptoskit_plugin_store_review"),
			Self::StylusInput => Some("leptoskit_native"),
			Self::WatchComplications
			| Self::WatchTiles
			| Self::WatchGlances
			| Self::DigitalCrown
			| Self::AlwaysOnDisplay
			| Self::WatchGestures => Some("leptoskit_watch"),
			Self::WatchWorkouts => Some("leptoskit_plugin_wearable"),
			Self::WatchCompanionSync => Some("leptoskit_plugin_watch_sync"),
			Self::SystemTray => Some("leptoskit_plugin_tray"),
			Self::AlwaysOnTopWindow
			| Self::CustomWindowChrome
			| Self::BackgroundOnlyApp
			| Self::TaskbarIntegration => Some("leptoskit_plugin_window_state"),
			Self::AutoUpdater => Some("leptoskit_plugin_updater"),
			Self::GlobalShortcuts => Some("leptoskit_plugin_global_shortcut"),
			Self::SingleInstance => Some("leptoskit_plugin_single_instance"),
			Self::Autostart => Some("leptoskit_plugin_autostart"),
			Self::NativeDialogs => Some("leptoskit_plugin_dialog"),
			Self::PowerMonitor => Some("leptoskit_plugin_power_monitor"),
			Self::ExtensionSidePanel
			| Self::ExtensionPopup
			| Self::ExtensionDevtoolsPanel
			| Self::ExtensionOffscreen
			| Self::ExtensionOmnibox
			| Self::ExtensionNativeMessaging
			| Self::ExtensionContentScripts => Some("leptoskit_extension"),
			Self::InAppPurchases
			| Self::AndroidPredictiveBack
			| Self::MediaSession
			| Self::SpeechRecognition
			| Self::DocumentAndBarcodeScanner
			| Self::WalletPasses
			| Self::NativePayments
			| Self::VoipCalling
			| Self::Geofencing
			| Self::MediaCasting
			| Self::AppClips
			| Self::SharePlay
			| Self::CloudKeyValueSync
			| Self::OnDeviceAi
			| Self::InAppUpdates
			| Self::AppAttestation
			| Self::ScreenTimeControls => None,
		}
	}

	/// The runtime plugin name to list under `native.plugins` in
	/// `leptoskit.toml`, or `None` for host-direct crates and unwired OS
	/// features.
	#[must_use]
	pub const fn runtime_plugin_name(self) -> Option<&'static str> {
		match self {
			Self::HomeWidgets
			| Self::LockScreenWidgets
			| Self::ControlCenterWidgets
			| Self::AndroidQuickSettingsTiles => Some("home-widgets"),
			Self::LiveActivities => Some("live-activities"),
			Self::AndroidIntents => Some("android_intent"),
			Self::AndroidBubbles
			| Self::MaterialYouDynamicColor
			| Self::AppIntents
			| Self::SystemSearchIndex
			| Self::Handoff
			| Self::IncomingShareTarget
			| Self::MultiWindowScenes => Some("app-shortcuts"),
			Self::PictureInPicture | Self::SystemOverlayWindow => Some("picture-in-picture"),
			Self::BackgroundExecution => Some("background"),
			Self::FoldablePosture => Some("posture"),
			Self::PushNotifications => Some("notifications"),
			Self::IconBadge => Some("badge"),
			Self::BluetoothLowEnergy => Some("ble"),
			Self::NfcReader => Some("nfc"),
			Self::HealthStore => Some("health"),
			Self::BiometricsAndHardwareKeys => Some("local-authentication"),
			Self::Passkeys => Some("passkeys"),
			Self::AugmentedReality => Some("xr"),
			Self::LivePhotos => Some("live-photo"),
			Self::AppTrackingTransparency => Some("tracking-transparency"),
			Self::AndroidExactAlarms => Some("android_alarm_manager"),
			Self::AndroidDeviceAdmin => Some("device-policy"),
			Self::AndroidAppUsage => Some("app-usage"),
			Self::ScreenCaptureProtection => Some("screen-capture"),
			Self::StoreReviewPrompt => Some("store-review"),
			Self::WatchWorkouts => Some("wearable"),
			Self::WatchCompanionSync => Some("watch-sync"),
			_ => None,
		}
	}

	/// Plain one-line description of this capability and its integration
	/// status.
	#[must_use]
	pub const fn summary(self) -> &'static str {
		if let Some(text) = self.mobile_summary() {
			return text;
		}
		match self {
			Self::AndroidPredictiveBack => {
				"Android 14+ predictive back gesture animation (not yet wired as a built-in \
				 plugin)."
			}
			Self::MediaSession => {
				"Lock Screen Now Playing and Android MediaSession controls (not yet wired as a \
				 built-in plugin)."
			}
			Self::SpeechRecognition => {
				"Live speech-to-text via SFSpeechRecognizer / SpeechRecognizer (not yet wired; \
				 speech plugin is TTS)."
			}
			Self::DocumentAndBarcodeScanner => {
				"VisionKit and ML Kit camera document/barcode scanning (not yet wired as a \
				 built-in plugin)."
			}
			Self::WalletPasses => {
				"Apple Wallet PassKit and Google Wallet passes (not yet wired as a built-in \
				 plugin)."
			}
			Self::NativePayments => {
				"Apple Pay and Google Pay native payment sheets (not yet wired as a built-in \
				 plugin)."
			}
			Self::VoipCalling => {
				"CallKit/PushKit and Android ConnectionService VoIP UI (not yet wired as a \
				 built-in plugin)."
			}
			Self::Geofencing => {
				"Background OS region monitoring and geofence triggers (not yet wired as a \
				 built-in plugin)."
			}
			Self::MediaCasting => {
				"AirPlay and Google Cast external routing (not yet wired as a built-in plugin)."
			}
			Self::AppClips => {
				"Apple App Clips and Google Play Instant entry points (not yet wired as a built-in \
				 plugin)."
			}
			Self::SharePlay => {
				"Apple GroupActivities SharePlay sessions (not yet wired as a built-in plugin)."
			}
			Self::CloudKeyValueSync => {
				"CloudKit, iCloud KVS, and Android Backup/Block Store sync (not yet wired as a \
				 built-in plugin)."
			}
			Self::OnDeviceAi => {
				"Apple Foundation Models / Core ML and Android Gemini Nano (not yet wired as a \
				 built-in plugin)."
			}
			Self::InAppUpdates => {
				"Google Play In-App Updates for native binaries (not yet wired as a built-in \
				 plugin)."
			}
			Self::AppAttestation => {
				"Apple App Attest and Play Integrity attestation (leptoskit_plugin_app_integrity \
				 is an Unavailable stub)."
			}
			Self::ScreenTimeControls => {
				"Apple FamilyControls and ManagedSettings shields (not yet wired as a built-in \
				 plugin)."
			}
			Self::WatchComplications => "watchOS and Wear OS watch face complications.",
			Self::WatchTiles => "Wear OS carousel tiles.",
			Self::WatchGlances => "watchOS Smart Stack glances.",
			Self::DigitalCrown => "Digital Crown and rotary bezel input events.",
			Self::AlwaysOnDisplay => "Watch Always-On Display and ambient lifecycle states.",
			Self::WatchWorkouts => "On-watch live biometric workout sessions.",
			Self::WatchCompanionSync => "Phone-to-watch companion messaging and queued sync.",
			Self::WatchGestures => "Watch wrist, double-tap, and action button gestures.",
			Self::DesktopMenuBar => "Native desktop application and window menu bars.",
			Self::DesktopContextMenu => "Native desktop context menus.",
			Self::SystemTray => "macOS menu bar extra, Windows tray, and Linux KSNI icons.",
			Self::AlwaysOnTopWindow => {
				"Floating, always-on-top, and click-through desktop windows."
			}
			Self::CustomWindowChrome => {
				"Frameless, transparent, vibrancy, Mica, and Acrylic window chrome."
			}
			Self::BackgroundOnlyApp => {
				"Menu-bar-only and background-only desktop application lifecycles."
			}
			Self::AutoUpdater => "Native desktop binary auto-updater feeds and hooks.",
			Self::GlobalShortcuts => "System-wide desktop keyboard shortcuts.",
			Self::SingleInstance => {
				"Single-instance OS process lock and second-launch activation handoff."
			}
			Self::Autostart => "Launch-at-login desktop startup registration.",
			Self::TaskbarIntegration => {
				"Dock and taskbar progress bars, badges, and user-attention alerts."
			}
			Self::NativeDialogs => "Native desktop open, save, directory, and message dialogs.",
			Self::PowerMonitor => "Desktop suspend, resume, lock, AC power, and idle events.",
			Self::ExtensionSidePanel => {
				"Browser extension side panel (chrome.sidePanel / browser.sidebarAction)."
			}
			Self::ExtensionPopup => "Browser extension toolbar popup surface.",
			Self::ExtensionDevtoolsPanel => "Browser extension DevTools panel surface.",
			Self::ExtensionOffscreen => "Chromium offscreen document background host.",
			Self::ExtensionOmnibox => "Browser address-bar omnibox keyword integration.",
			Self::ExtensionNativeMessaging => {
				"Browser extension native messaging host communication."
			}
			Self::ExtensionContentScripts => {
				"Browser extension content script injection into web pages."
			}
			_ => unreachable!(),
		}
	}

	const fn mobile_summary(self) -> Option<&'static str> {
		Some(match self {
			Self::IosMenuBar => {
				"iOS/iPadOS UIMenuBuilder menu bars and hardware keyboard commands."
			}
			Self::HomeWidgets => {
				"Home-screen widgets and timelines via WidgetKit and Android App Widgets."
			}
			Self::LockScreenWidgets => {
				"iOS Lock Screen and StandBy accessory widgets via WidgetKit timelines."
			}
			Self::ControlCenterWidgets => {
				"iOS 18+ Control Center toggles and actions via ControlWidget reloads."
			}
			Self::LiveActivities => {
				"iOS ActivityKit Live Activities, Dynamic Island, push-to-start, and Android 16+ \
				 Live Updates."
			}
			Self::AndroidQuickSettingsTiles => {
				"Android Quick Settings TileService state publishing and reading."
			}
			Self::AndroidBubbles => {
				"Android conversation bubbles (typed BubbleConfig descriptors; host posts \
				 notifications)."
			}
			Self::AndroidIntents => {
				"Explicit/implicit Android Intents, choosers, broadcasts, and intent: URIs."
			}
			Self::MaterialYouDynamicColor => {
				"Android Material You Monet dynamic color roles (typed descriptors; host reads \
				 theme)."
			}
			Self::AppIntents => {
				"Launcher quick actions/shortcuts and typed App Intent / Siri Shortcut descriptors."
			}
			Self::SystemSearchIndex => {
				"Apple CoreSpotlight indexing wired on iOS/macOS; Android AppSearch is \
				 host-supplied."
			}
			Self::Handoff => "Apple NSUserActivity Handoff and Continuity publishing on iOS/macOS.",
			Self::IncomingShareTarget => {
				"Incoming share intake (Android ACTION_SEND wired; iOS Share Extension feeds \
				 noteIncomingShare)."
			}
			Self::InAppPurchases => {
				"StoreKit 2 and Google Play Billing in-app purchases (not yet wired as a built-in \
				 plugin)."
			}
			Self::PictureInPicture => {
				"Android Picture-in-Picture leases and iOS PiP capability reporting."
			}
			Self::SystemOverlayWindow => {
				"Android SYSTEM_ALERT_WINDOW draw-over-other-apps permission and overlay leases."
			}
			Self::BackgroundExecution => {
				"Android foreground services/WorkManager and iOS BGTaskScheduler tasks."
			}
			Self::FoldablePosture => {
				"Foldable hinge state, angle, and dual-pane/tabletop layout posture."
			}
			Self::CodePush => "Signed over-the-air Wasm bundle updates and binary deltas.",
			Self::PushNotifications => {
				"Local notifications plus APNs (iOS/macOS) and FCM (Android) push tokens."
			}
			Self::IconBadge => "App icon and Dock badge count updates.",
			Self::BluetoothLowEnergy => {
				"BLE GATT central scanning, connections, and characteristic notifications."
			}
			Self::NfcReader => "Core NFC and Android NFC NDEF text/URI tag reading.",
			Self::HealthStore => {
				"HealthKit (iOS/watchOS), Health Connect (Android), and HarmonyOS Health reads."
			}
			Self::BiometricsAndHardwareKeys => {
				"Face ID/Touch ID/BiometricPrompt plus Secure Enclave and StrongBox keys \
				 (device-keys)."
			}
			Self::Passkeys => "WebAuthn passkeys and Sign in with Apple authentication.",
			Self::AugmentedReality => "ARKit, ARCore, OpenXR, and WebXR tracking sessions.",
			Self::LivePhotos => "Apple PHLivePhotoView playback for ph:// photo assets.",
			Self::AppTrackingTransparency => {
				"iOS ATTrackingManager authorization status and prompt."
			}
			Self::AndroidExactAlarms => {
				"Android AlarmManager exact, wake-up, reboot-persisted, and headless alarms."
			}
			Self::AndroidDeviceAdmin => {
				"Android DevicePolicyManager lock, password, camera, and confirmed wipe actions."
			}
			Self::AndroidAppUsage => {
				"Android UsageStatsManager screen-time queries and Apple lock/unlock events."
			}
			Self::ScreenCaptureProtection => {
				"Android FLAG_SECURE window protection and iOS capture/screenshot observation."
			}
			Self::StoreReviewPrompt => "StoreKit and Google Play in-app review prompts.",
			Self::MultiWindowScenes => {
				"iPadOS Stage Manager/Split View MultiSceneConfig descriptors and Android \
				 multi-window."
			}
			Self::StylusInput => {
				"Apple Pencil and Android stylus tool detection and PencilPreference descriptors."
			}
			_ => return None,
		})
	}
}

impl fmt::Display for Capability {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.as_str())
	}
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

	/// True when `capability` is available (`Supported` or `Conditional`) on
	/// this platform.
	#[must_use]
	pub const fn supports(self, capability: Capability) -> bool {
		self.capabilities().is_available(capability)
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
	/// Iterates over every [`Capability`] that is either
	/// [`CapabilitySupport::Supported`] or [`CapabilitySupport::Conditional`]
	/// on this platform.
	pub fn available_capabilities(self) -> impl Iterator<Item = Capability> {
		Capability::ALL
			.iter()
			.copied()
			.filter(move |&capability| self.is_available(capability))
	}

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
	pub const fn support(self, capability: Capability) -> CapabilitySupport {
		match self.platform {
			TargetPlatform::Ios => support_ios(capability),
			TargetPlatform::Android => support_android(capability),
			TargetPlatform::Watchos => support_watchos(capability),
			TargetPlatform::Wearos => support_wearos(capability),
			TargetPlatform::Macos => support_macos(capability),
			TargetPlatform::Windows => support_windows(capability),
			TargetPlatform::Linux => support_linux(capability),
			TargetPlatform::Extension => support_extension(capability),
			TargetPlatform::Web => support_web(capability),
			TargetPlatform::Harmonyos => support_harmonyos(capability),
			TargetPlatform::Quest => support_quest(capability),
			TargetPlatform::Tui | TargetPlatform::Server | TargetPlatform::Shared => {
				CapabilitySupport::Unavailable
			}
		}
	}

	/// Iterates over every [`Capability`] with [`CapabilitySupport::Supported`]
	/// on this platform.
	pub fn supported_capabilities(self) -> impl Iterator<Item = Capability> {
		Capability::ALL
			.iter()
			.copied()
			.filter(move |&capability| self.is_supported(capability))
	}
}

const fn support_ios(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Conditional;
	use CapabilitySupport::Supported;
	use CapabilitySupport::Unavailable;

	match capability {
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
		| C::CodePush
		| C::PushNotifications
		| C::IconBadge
		| C::BluetoothLowEnergy
		| C::NfcReader
		| C::HealthStore
		| C::BiometricsAndHardwareKeys
		| C::Passkeys
		| C::AugmentedReality
		| C::LivePhotos
		| C::AppTrackingTransparency
		| C::ScreenCaptureProtection
		| C::StoreReviewPrompt
		| C::StylusInput
		| C::MediaSession
		| C::SpeechRecognition
		| C::DocumentAndBarcodeScanner
		| C::WalletPasses
		| C::NativePayments
		| C::VoipCalling
		| C::Geofencing
		| C::MediaCasting
		| C::CloudKeyValueSync => Supported,
		C::PictureInPicture
		| C::FoldablePosture
		| C::DesktopContextMenu
		| C::AndroidAppUsage
		| C::MultiWindowScenes
		| C::AppClips
		| C::SharePlay
		| C::OnDeviceAi
		| C::AppAttestation
		| C::ScreenTimeControls => Conditional,
		_ => Unavailable,
	}
}

const fn support_android(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Conditional;
	use CapabilitySupport::Supported;
	use CapabilitySupport::Unavailable;

	match capability {
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
		| C::CodePush
		| C::PushNotifications
		| C::IconBadge
		| C::BluetoothLowEnergy
		| C::NfcReader
		| C::HealthStore
		| C::BiometricsAndHardwareKeys
		| C::Passkeys
		| C::AugmentedReality
		| C::AndroidExactAlarms
		| C::ScreenCaptureProtection
		| C::StoreReviewPrompt
		| C::StylusInput
		| C::MediaSession
		| C::SpeechRecognition
		| C::DocumentAndBarcodeScanner
		| C::WalletPasses
		| C::NativePayments
		| C::VoipCalling
		| C::Geofencing
		| C::MediaCasting
		| C::InAppUpdates => Supported,
		C::LiveActivities
		| C::AndroidDeviceAdmin
		| C::AndroidAppUsage
		| C::MultiWindowScenes
		| C::AndroidPredictiveBack
		| C::AppClips
		| C::CloudKeyValueSync
		| C::OnDeviceAi
		| C::AppAttestation => Conditional,
		_ => Unavailable,
	}
}

const fn support_watchos(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Supported;
	use CapabilitySupport::Unavailable;

	match capability {
		C::WatchComplications
		| C::WatchGlances
		| C::DigitalCrown
		| C::AlwaysOnDisplay
		| C::WatchWorkouts
		| C::WatchCompanionSync
		| C::WatchGestures
		| C::Handoff
		| C::BluetoothLowEnergy
		| C::HealthStore
		| C::WalletPasses
		| C::NativePayments
		| C::CloudKeyValueSync => Supported,
		_ => Unavailable,
	}
}

const fn support_wearos(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Supported;
	use CapabilitySupport::Unavailable;

	match capability {
		C::WatchComplications
		| C::WatchTiles
		| C::DigitalCrown
		| C::AlwaysOnDisplay
		| C::WatchWorkouts
		| C::WatchCompanionSync
		| C::WatchGestures
		| C::BluetoothLowEnergy
		| C::WalletPasses
		| C::NativePayments => Supported,
		_ => Unavailable,
	}
}

const fn support_macos(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Conditional;
	use CapabilitySupport::Supported;
	use CapabilitySupport::Unavailable;

	match capability {
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
		| C::SystemSearchIndex
		| C::PushNotifications
		| C::IconBadge
		| C::BiometricsAndHardwareKeys
		| C::Passkeys
		| C::LivePhotos
		| C::StoreReviewPrompt
		| C::MediaSession
		| C::SpeechRecognition
		| C::NativePayments
		| C::MediaCasting
		| C::CloudKeyValueSync => Supported,
		C::InAppPurchases
		| C::AppIntents
		| C::AndroidAppUsage
		| C::SharePlay
		| C::OnDeviceAi
		| C::ScreenTimeControls => Conditional,
		_ => Unavailable,
	}
}

const fn support_windows(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Conditional;
	use CapabilitySupport::Supported;
	use CapabilitySupport::Unavailable;

	match capability {
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
		| C::MediaSession => Supported,
		C::StylusInput => Conditional,
		_ => Unavailable,
	}
}

const fn support_linux(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Conditional;
	use CapabilitySupport::Supported;
	use CapabilitySupport::Unavailable;

	match capability {
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
		| C::PowerMonitor
		| C::MediaSession => Supported,
		// Global shortcuts work on X11; Wayland needs portal support.
		// Menu bars depend on desktop environment / tray KSNI.
		C::GlobalShortcuts
		| C::DesktopMenuBar
		| C::PushNotifications
		| C::IconBadge
		| C::StylusInput => Conditional,
		_ => Unavailable,
	}
}

const fn support_extension(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Conditional;
	use CapabilitySupport::Supported;
	use CapabilitySupport::Unavailable;

	match capability {
		C::ExtensionPopup | C::ExtensionContentScripts => Supported,
		// Side panel, DevTools, offscreen, omnibox, and native messaging vary
		// by browser family (Chromium vs Firefox vs Safari).
		C::ExtensionSidePanel
		| C::ExtensionDevtoolsPanel
		| C::ExtensionOffscreen
		| C::ExtensionOmnibox
		| C::ExtensionNativeMessaging => Conditional,
		_ => Unavailable,
	}
}

const fn support_web(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Conditional;
	use CapabilitySupport::Supported;
	use CapabilitySupport::Unavailable;

	match capability {
		C::MediaSession => Supported,
		C::FoldablePosture
		| C::PictureInPicture
		| C::PushNotifications
		| C::IconBadge
		| C::Passkeys
		| C::AugmentedReality
		| C::StylusInput
		| C::SpeechRecognition
		| C::NativePayments
		| C::MediaCasting => Conditional,
		_ => Unavailable,
	}
}

const fn support_harmonyos(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Conditional;
	use CapabilitySupport::Unavailable;

	match capability {
		C::FoldablePosture | C::HomeWidgets | C::BluetoothLowEnergy | C::HealthStore => Conditional,
		_ => Unavailable,
	}
}

const fn support_quest(capability: Capability) -> CapabilitySupport {
	use Capability as C;
	use CapabilitySupport::Conditional;
	use CapabilitySupport::Unavailable;

	match capability {
		C::AndroidIntents | C::BackgroundExecution | C::AugmentedReality => Conditional,
		_ => Unavailable,
	}
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeSet;

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
	fn capabilities_distinguish_mobile_features() {
		let ios = PlatformCapabilities::for_platform(TargetPlatform::Ios);
		assert!(ios.is_supported(Capability::IosMenuBar));
		assert!(ios.is_supported(Capability::LiveActivities));
		assert!(ios.is_supported(Capability::ControlCenterWidgets));
		assert!(ios.is_supported(Capability::HealthStore));
		assert!(ios.is_supported(Capability::AppTrackingTransparency));
		assert!(TargetPlatform::Ios.supports(Capability::LivePhotos));
		assert!(!ios.is_available(Capability::AndroidQuickSettingsTiles));
		assert!(!ios.is_available(Capability::AndroidExactAlarms));

		let android = PlatformCapabilities::for_platform(TargetPlatform::Android);
		assert!(android.is_supported(Capability::AndroidQuickSettingsTiles));
		assert!(android.is_supported(Capability::AndroidBubbles));
		assert!(android.is_supported(Capability::SystemOverlayWindow));
		assert!(android.is_supported(Capability::AndroidExactAlarms));
		assert!(android.is_supported(Capability::ScreenCaptureProtection));
		assert_eq!(
			android.support(Capability::LiveActivities),
			CapabilitySupport::Conditional
		);
		assert_eq!(
			android.support(Capability::AndroidPredictiveBack),
			CapabilitySupport::Conditional
		);
		assert!(!android.is_available(Capability::Handoff));
		assert!(!android.is_available(Capability::AppTrackingTransparency));
	}

	#[test]
	fn capabilities_distinguish_desktop_watch_and_extension_features() {
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

	#[test]
	fn capability_catalog_metadata_is_unique_and_honest() {
		let mut seen_names = BTreeSet::new();
		for &capability in Capability::ALL {
			let name = capability.as_str();
			assert_ne!(name, "");
			assert!(
				seen_names.insert(name),
				"duplicate capability name `{name}`"
			);
			assert_eq!(capability.to_string(), name);
			assert_ne!(capability.summary(), "", "empty summary for `{name}`");
			if capability.runtime_plugin_name().is_some() {
				assert!(
					capability.has_builtin_plugin(),
					"runtime plugin `{name}` must report a plugin_crate"
				);
			}
			if capability.cargo_feature().is_some() {
				assert!(
					capability.has_builtin_plugin(),
					"cargo feature `{name}` must report a plugin_crate"
				);
			}
		}

		// Built-in wired plugins report crate, runtime name, and Cargo feature.
		assert_eq!(
			Capability::LiveActivities.plugin_crate(),
			Some("leptoskit_plugin_live_activities")
		);
		assert_eq!(
			Capability::LiveActivities.runtime_plugin_name(),
			Some("live-activities")
		);
		assert_eq!(
			Capability::LiveActivities.cargo_feature(),
			Some("live-activities")
		);
		assert!(Capability::LiveActivities.has_builtin_plugin());

		assert_eq!(
			Capability::HomeWidgets.plugin_crate(),
			Some("leptoskit_plugin_home_widgets")
		);
		assert_eq!(
			Capability::HomeWidgets.runtime_plugin_name(),
			Some("home-widgets")
		);
		assert_eq!(
			Capability::AndroidExactAlarms.runtime_plugin_name(),
			Some("android_alarm_manager")
		);
		assert_eq!(
			Capability::BluetoothLowEnergy.plugin_crate(),
			Some("leptoskit_plugin_ble")
		);
		assert_eq!(
			Capability::HealthStore.plugin_crate(),
			Some("leptoskit_plugin_health")
		);

		// Unwired OS capabilities honestly report no built-in plugin yet.
		for unwired in [
			Capability::InAppPurchases,
			Capability::NativePayments,
			Capability::WalletPasses,
			Capability::VoipCalling,
			Capability::MediaSession,
			Capability::SpeechRecognition,
			Capability::DocumentAndBarcodeScanner,
			Capability::OnDeviceAi,
			Capability::AppAttestation,
			Capability::AndroidPredictiveBack,
		] {
			assert!(
				!unwired.has_builtin_plugin(),
				"`{unwired}` should report has_builtin_plugin() == false"
			);
			assert_eq!(unwired.plugin_crate(), None);
			assert_eq!(unwired.runtime_plugin_name(), None);
			assert_eq!(unwired.cargo_feature(), None);
		}
	}

	#[test]
	fn platform_capability_iterators_match_queries() {
		let ios = PlatformCapabilities::for_platform(TargetPlatform::Ios);
		let ios_supported: Vec<_> = ios.supported_capabilities().collect();
		let ios_available: Vec<_> = ios.available_capabilities().collect();
		assert!(ios_supported.contains(&Capability::LiveActivities));
		assert!(ios_supported.contains(&Capability::HomeWidgets));
		assert!(!ios_supported.contains(&Capability::MultiWindowScenes));
		assert!(ios_available.contains(&Capability::MultiWindowScenes));
		assert!(ios_available.len() > ios_supported.len());

		let android = PlatformCapabilities::for_platform(TargetPlatform::Android);
		let android_available: Vec<_> = android.available_capabilities().collect();
		assert!(android_available.contains(&Capability::LiveActivities));
		assert!(android_available.contains(&Capability::AndroidQuickSettingsTiles));
		assert!(!android_available.contains(&Capability::IosMenuBar));
	}

	#[cfg(feature = "serde")]
	#[test]
	fn capability_as_str_matches_serde_kebab_case() {
		for &capability in Capability::ALL {
			let serialized = serde_json::to_string(&capability).unwrap();
			assert_eq!(serialized, format!("\"{}\"", capability.as_str()));
		}
	}
}
