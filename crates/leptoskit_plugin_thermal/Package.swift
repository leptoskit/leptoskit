// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitThermal",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitThermal", targets: ["LeptosKitThermal"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitThermal",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitThermalTests",
            dependencies: ["LeptosKitThermal", .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple-tests"
        )
    ]
)
