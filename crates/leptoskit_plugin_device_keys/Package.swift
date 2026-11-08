// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitDeviceKeys",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitDeviceKeys", targets: ["LeptosKitDeviceKeys"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitDeviceKeys",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitDeviceKeysTests",
            dependencies: [
                "LeptosKitDeviceKeys",
                .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
            ],
            path: "apple-tests"
        )
    ]
)
