// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitRecorder",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitRecorder", targets: ["LeptosKitRecorder"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitRecorder",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(name: "LeptosKitRecorderTests", dependencies: ["LeptosKitRecorder"], path: "apple-tests")
    ]
)
