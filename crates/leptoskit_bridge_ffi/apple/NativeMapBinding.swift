import Foundation
import MapKit

/// Lookup hook the `maps` plugin installs so `NativeMapView` can attach an
/// `MKMapView` by host-owned id without linking the plugin into every app.
@MainActor
public enum NativeMapBinding {
    /// Returns the map view for `id`, or `nil` when no plugin owns it.
    public static var mapView: ((UInt32) -> MKMapView?)?
}
