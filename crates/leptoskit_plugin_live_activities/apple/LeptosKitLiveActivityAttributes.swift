import Foundation

#if canImport(ActivityKit) && os(iOS)
@preconcurrency import ActivityKit

/// Portable Live Activity attributes shared by the plugin and Widget Extensions.
///
/// Apps that want Lock Screen or Dynamic Island UI must include a Widget
/// Extension that renders this type. The plugin starts and updates activities;
/// it does not ship widget chrome.
@available(iOS 16.1, *)
public struct LeptosKitLiveActivityAttributes: ActivityAttributes {
    public struct ContentState: Codable, Hashable, Sendable {
        public var fields: [String: String]

        public init(fields: [String: String]) {
            self.fields = fields
        }

        /// Dynamic Island compact leading slot (`_lk_di_cl`), if set.
        public var compactLeading: String? { fields["_lk_di_cl"] }
        /// Dynamic Island compact trailing slot (`_lk_di_ct`), if set.
        public var compactTrailing: String? { fields["_lk_di_ct"] }
        /// Dynamic Island minimal badge slot (`_lk_di_min`), if set.
        public var minimal: String? { fields["_lk_di_min"] }
        /// Dynamic Island expanded leading slot (`_lk_di_el`), if set.
        public var expandedLeading: String? { fields["_lk_di_el"] }
        /// Dynamic Island expanded trailing slot (`_lk_di_et`), if set.
        public var expandedTrailing: String? { fields["_lk_di_et"] }
        /// Dynamic Island expanded center headline (`_lk_di_ec`), if set.
        public var expandedCenter: String? { fields["_lk_di_ec"] }
        /// Dynamic Island expanded bottom region (`_lk_di_eb`), if set.
        public var expandedBottom: String? { fields["_lk_di_eb"] }
        /// Optional `#RRGGBB` Dynamic Island keyline tint (`_lk_di_tint`).
        public var keylineTintHex: String? { fields["_lk_di_tint"] }
    }

    /// Application-chosen activity identifier.
    public var id: String
    /// Grouping key for the Widget Extension.
    public var kind: String
    /// Static title stored for the lifetime of the activity.
    public var title: String

    public init(id: String, kind: String, title: String) {
        self.id = id
        self.kind = kind
        self.title = title
    }
}
#endif
