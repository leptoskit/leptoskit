// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitAppIntegrity",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitAppIntegrity", targets: ["LeptosKitAppIntegrity"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(name: "LeptosKitAppIntegrity", dependencies: [
            .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
        ], path: "apple"),
        .testTarget(
            name: "AppIntegrityTests",
            dependencies: ["LeptosKitAppIntegrity"],
            path: "apple-tests"
        )
    ]
)
