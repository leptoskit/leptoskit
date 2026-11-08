import Foundation
import LeptosKitBridge
import MapKit
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Host-owned `MKMapView` registry with camera and marker updates.
@MainActor
public final class MapsPlugin: NativePlugin {
    public let name = "maps"
    public let methods: [UInt32] = [0, 1, 2, 3, 4]

    private var sessions: [UInt32: Session] = [:]
    private var nextId: UInt32 = 1
    private var closed = false

    private enum Method: UInt32 {
        case create = 0
        case setCamera = 1
        case setMarkers = 2
        case status = 3
        case release = 4
    }

    private final class Session {
        let mapView: MKMapView
        var camera: MapCamera
        var markers: [MapMarker] = []

        init(mapView: MKMapView, camera: MapCamera) {
            self.mapView = mapView
            self.camera = camera
        }
    }

    public init() {
        NativeMapBinding.mapView = { [weak self] id in self?.sessions[id]?.mapView }
    }

    public func invoke(method: UInt32, payload: NativeBuffer) async throws -> Data {
        guard !closed else { throw PluginError.cancelled }
        guard let method = Method(rawValue: method) else { throw PluginError.unavailable }

        switch method {
        case .create:
            try MapsWire.create(payload)
            return try create()
        case .setCamera:
            let request = try MapsWire.setCamera(payload)
            try setCamera(request.id, request.camera)
            return Data()
        case .setMarkers:
            let request = try MapsWire.setMarkers(payload)
            try setMarkers(request.id, request.markers)
            return Data()
        case .status:
            return try status(MapsWire.session(payload))
        case .release:
            try release(MapsWire.session(payload))
            return Data()
        }
    }

    public func close() {
        closed = true
        for id in Array(sessions.keys) {
            try? release(id)
        }
        NativeMapBinding.mapView = nil
    }

    private func create() throws -> Data {
        let mapView = MKMapView(frame: .zero)
        let camera = MapCamera.default
        mapView.setRegion(camera.region, animated: false)
        let id = nextId
        nextId = nextId &+ 1
        if nextId == 0 { nextId = 1 }
        sessions[id] = Session(mapView: mapView, camera: camera)
        return MapsWire.sessionId(id)
    }

    private func setCamera(_ id: UInt32, _ camera: MapCamera) throws {
        guard let session = sessions[id] else { throw PluginError.invalidArgument }
        session.camera = camera
        session.mapView.setRegion(camera.region, animated: true)
    }

    private func setMarkers(_ id: UInt32, _ markers: [MapMarker]) throws {
        guard let session = sessions[id] else { throw PluginError.invalidArgument }
        session.markers = markers
        session.mapView.removeAnnotations(session.mapView.annotations)
        for marker in markers {
            let annotation = MKPointAnnotation()
            annotation.coordinate = CLLocationCoordinate2D(
                latitude: marker.latitude, longitude: marker.longitude)
            if !marker.title.isEmpty {
                annotation.title = marker.title
            }
            session.mapView.addAnnotation(annotation)
        }
    }

    private func status(_ id: UInt32) throws -> Data {
        guard let session = sessions[id] else { throw PluginError.invalidArgument }
        return MapsWire.status(
            camera: session.camera,
            markerCount: UInt32(session.markers.count),
            active: true
        )
    }

    private func release(_ id: UInt32) throws {
        guard let session = sessions.removeValue(forKey: id) else {
            throw PluginError.invalidArgument
        }
        session.mapView.removeAnnotations(session.mapView.annotations)
        session.mapView.removeFromSuperview()
    }
}
