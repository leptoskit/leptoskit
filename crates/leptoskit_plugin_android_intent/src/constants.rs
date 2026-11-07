//! Android constants, spelled as the platform defines them.

pub mod action {
	//! Common `Intent` actions. Any other action string works too.
	//!
	//! Upstream's aliases map to these: `action_view` is [`VIEW`], `settings`
	//! is [`SETTINGS`], `action_voice` is [`VOICE_COMMAND`],
	//! `action_location_source_settings` is [`LOCATION_SOURCE_SETTINGS`] and
	//! `action_application_details_settings` is
	//! [`APPLICATION_DETAILS_SETTINGS`].

	/// `Intent.ACTION_VIEW`.
	pub const VIEW: &str = "android.intent.action.VIEW";
	/// `Intent.ACTION_SEND`.
	pub const SEND: &str = "android.intent.action.SEND";
	/// `Intent.ACTION_SENDTO`.
	pub const SENDTO: &str = "android.intent.action.SENDTO";
	/// `Intent.ACTION_DIAL`. Opens the dialer without placing a call.
	pub const DIAL: &str = "android.intent.action.DIAL";
	/// `Intent.ACTION_MAIN`.
	pub const MAIN: &str = "android.intent.action.MAIN";
	/// `Intent.ACTION_EDIT`.
	pub const EDIT: &str = "android.intent.action.EDIT";
	/// `Intent.ACTION_PICK`.
	pub const PICK: &str = "android.intent.action.PICK";
	/// `Intent.ACTION_WEB_SEARCH`.
	pub const WEB_SEARCH: &str = "android.intent.action.WEB_SEARCH";
	/// `Intent.ACTION_VOICE_COMMAND`.
	pub const VOICE_COMMAND: &str = "android.intent.action.VOICE_COMMAND";
	/// `Settings.ACTION_SETTINGS`.
	pub const SETTINGS: &str = "android.settings.SETTINGS";
	/// `Settings.ACTION_APPLICATION_DETAILS_SETTINGS`; pair it with
	/// `data("package:<your application id>")`.
	pub const APPLICATION_DETAILS_SETTINGS: &str = "android.settings.APPLICATION_DETAILS_SETTINGS";
	/// `Settings.ACTION_LOCATION_SOURCE_SETTINGS`.
	pub const LOCATION_SOURCE_SETTINGS: &str = "android.settings.LOCATION_SOURCE_SETTINGS";
	/// `Settings.ACTION_WIFI_SETTINGS`.
	pub const WIFI_SETTINGS: &str = "android.settings.WIFI_SETTINGS";
	/// `Settings.ACTION_BLUETOOTH_SETTINGS`.
	pub const BLUETOOTH_SETTINGS: &str = "android.settings.BLUETOOTH_SETTINGS";
	/// `Settings.ACTION_APP_NOTIFICATION_SETTINGS`; pair it with the
	/// [`crate::extra::APP_PACKAGE`] extra.
	pub const APP_NOTIFICATION_SETTINGS: &str = "android.settings.APP_NOTIFICATION_SETTINGS";
}

/// Common `Intent` categories.
pub mod category {
	/// `Intent.CATEGORY_DEFAULT`.
	pub const DEFAULT: &str = "android.intent.category.DEFAULT";
	/// `Intent.CATEGORY_BROWSABLE`.
	pub const BROWSABLE: &str = "android.intent.category.BROWSABLE";
	/// `Intent.CATEGORY_LAUNCHER`.
	pub const LAUNCHER: &str = "android.intent.category.LAUNCHER";
	/// `Intent.CATEGORY_HOME`.
	pub const HOME: &str = "android.intent.category.HOME";
	/// `Intent.CATEGORY_OPENABLE`.
	pub const OPENABLE: &str = "android.intent.category.OPENABLE";
	/// `Intent.CATEGORY_APP_BROWSER`.
	pub const APP_BROWSER: &str = "android.intent.category.APP_BROWSER";
	/// `Intent.CATEGORY_APP_EMAIL`.
	pub const APP_EMAIL: &str = "android.intent.category.APP_EMAIL";
	/// `Intent.CATEGORY_APP_MAPS`.
	pub const APP_MAPS: &str = "android.intent.category.APP_MAPS";
}

/// Common extra keys.
pub mod extra {
	/// `Intent.EXTRA_TEXT`.
	pub const TEXT: &str = "android.intent.extra.TEXT";
	/// `Intent.EXTRA_SUBJECT`.
	pub const SUBJECT: &str = "android.intent.extra.SUBJECT";
	/// `Intent.EXTRA_EMAIL`, a string array of recipients.
	pub const EMAIL: &str = "android.intent.extra.EMAIL";
	/// `Intent.EXTRA_CC`, a string array.
	pub const CC: &str = "android.intent.extra.CC";
	/// `Intent.EXTRA_BCC`, a string array.
	pub const BCC: &str = "android.intent.extra.BCC";
	/// `Intent.EXTRA_TITLE`.
	pub const TITLE: &str = "android.intent.extra.TITLE";
	/// `Settings.EXTRA_APP_PACKAGE`.
	pub const APP_PACKAGE: &str = "android.provider.extra.APP_PACKAGE";
}

/// `Intent` flags `android_intent_plus` exposes, minus the four
/// `FLAG_GRANT_*_URI_PERMISSION` flags this plugin always refuses (see
/// [`crate::GRANT_URI_PERMISSION_FLAGS`]).
///
/// Activity and receiver flags share bit values, as they do in Android:
/// [`RECEIVER_FOREGROUND`](flag::RECEIVER_FOREGROUND) and
/// [`ACTIVITY_NEW_TASK`](flag::ACTIVITY_NEW_TASK) are the same bit, and
/// mean whichever applies to the method you call.
pub mod flag {
	use crate::Flag;

	/// `FLAG_ACTIVITY_BROUGHT_TO_FRONT`.
	pub const ACTIVITY_BROUGHT_TO_FRONT: Flag = Flag(0x0040_0000);
	/// `FLAG_ACTIVITY_CLEAR_TASK`.
	pub const ACTIVITY_CLEAR_TASK: Flag = Flag(0x0000_8000);
	/// `FLAG_ACTIVITY_CLEAR_TOP`.
	pub const ACTIVITY_CLEAR_TOP: Flag = Flag(0x0400_0000);
	/// `FLAG_ACTIVITY_CLEAR_WHEN_TASK_RESET`, deprecated for
	/// [`ACTIVITY_NEW_DOCUMENT`], which is the same bit.
	pub const ACTIVITY_CLEAR_WHEN_TASK_RESET: Flag = Flag(0x0008_0000);
	/// `FLAG_ACTIVITY_EXCLUDE_FROM_RECENTS`.
	pub const ACTIVITY_EXCLUDE_FROM_RECENTS: Flag = Flag(0x0080_0000);
	/// `FLAG_ACTIVITY_FORWARD_RESULT`.
	pub const ACTIVITY_FORWARD_RESULT: Flag = Flag(0x0200_0000);
	/// `FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY`.
	pub const ACTIVITY_LAUNCHED_FROM_HISTORY: Flag = Flag(0x0010_0000);
	/// `FLAG_ACTIVITY_LAUNCH_ADJACENT`.
	pub const ACTIVITY_LAUNCH_ADJACENT: Flag = Flag(0x0000_1000);
	/// `FLAG_ACTIVITY_MATCH_EXTERNAL`.
	pub const ACTIVITY_MATCH_EXTERNAL: Flag = Flag(0x0000_0800);
	/// `FLAG_ACTIVITY_MULTIPLE_TASK`.
	pub const ACTIVITY_MULTIPLE_TASK: Flag = Flag(0x0800_0000);
	/// `FLAG_ACTIVITY_NEW_DOCUMENT`.
	pub const ACTIVITY_NEW_DOCUMENT: Flag = Flag(0x0008_0000);
	/// `FLAG_ACTIVITY_NEW_TASK`.
	pub const ACTIVITY_NEW_TASK: Flag = Flag(0x1000_0000);
	/// `FLAG_ACTIVITY_NO_ANIMATION`.
	pub const ACTIVITY_NO_ANIMATION: Flag = Flag(0x0001_0000);
	/// `FLAG_ACTIVITY_NO_HISTORY`.
	pub const ACTIVITY_NO_HISTORY: Flag = Flag(0x4000_0000);
	/// `FLAG_ACTIVITY_NO_USER_ACTION`.
	pub const ACTIVITY_NO_USER_ACTION: Flag = Flag(0x0004_0000);
	/// `FLAG_ACTIVITY_PREVIOUS_IS_TOP`.
	pub const ACTIVITY_PREVIOUS_IS_TOP: Flag = Flag(0x0100_0000);
	/// `FLAG_ACTIVITY_REORDER_TO_FRONT`.
	pub const ACTIVITY_REORDER_TO_FRONT: Flag = Flag(0x0002_0000);
	/// `FLAG_ACTIVITY_RESET_TASK_IF_NEEDED`.
	pub const ACTIVITY_RESET_TASK_IF_NEEDED: Flag = Flag(0x0020_0000);
	/// `FLAG_ACTIVITY_RETAIN_IN_RECENTS`.
	pub const ACTIVITY_RETAIN_IN_RECENTS: Flag = Flag(0x0000_2000);
	/// `FLAG_ACTIVITY_SINGLE_TOP`.
	pub const ACTIVITY_SINGLE_TOP: Flag = Flag(0x2000_0000);
	/// `FLAG_ACTIVITY_TASK_ON_HOME`.
	pub const ACTIVITY_TASK_ON_HOME: Flag = Flag(0x0000_4000);
	/// `FLAG_DEBUG_LOG_RESOLUTION`.
	pub const DEBUG_LOG_RESOLUTION: Flag = Flag(0x0000_0008);
	/// `FLAG_EXCLUDE_STOPPED_PACKAGES`.
	pub const EXCLUDE_STOPPED_PACKAGES: Flag = Flag(0x0000_0010);
	/// `FLAG_FROM_BACKGROUND`.
	pub const FROM_BACKGROUND: Flag = Flag(0x0000_0004);
	/// `FLAG_INCLUDE_STOPPED_PACKAGES`.
	pub const INCLUDE_STOPPED_PACKAGES: Flag = Flag(0x0000_0020);
	/// `FLAG_RECEIVER_FOREGROUND`.
	pub const RECEIVER_FOREGROUND: Flag = Flag(0x1000_0000);
	/// `FLAG_RECEIVER_NO_ABORT`.
	pub const RECEIVER_NO_ABORT: Flag = Flag(0x0800_0000);
	/// `FLAG_RECEIVER_REGISTERED_ONLY`.
	pub const RECEIVER_REGISTERED_ONLY: Flag = Flag(0x4000_0000);
	/// `FLAG_RECEIVER_REPLACE_PENDING`.
	pub const RECEIVER_REPLACE_PENDING: Flag = Flag(0x2000_0000);
	/// `FLAG_RECEIVER_VISIBLE_TO_INSTANT_APPS`.
	pub const RECEIVER_VISIBLE_TO_INSTANT_APPS: Flag = Flag(0x0020_0000);
}
