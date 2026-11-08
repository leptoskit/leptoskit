// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitMaps",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitMaps", targets: ["LeptosKitMaps"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitMaps",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitMapsTests",
            dependencies: ["LeptosKitMaps"],
            path: "apple-tests"
        )
    ]
)
