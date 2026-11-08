// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitBackgroundTask",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitBackgroundTask", targets: ["LeptosKitBackgroundTask"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitBackgroundTask",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitBackgroundTaskTests",
            dependencies: ["LeptosKitBackgroundTask"],
            path: "apple-tests"
        )
    ]
)
