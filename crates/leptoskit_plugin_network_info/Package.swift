// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitNetworkInfo",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitNetworkInfo", targets: ["LeptosKitNetworkInfo"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitNetworkInfo",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitNetworkInfoTests",
            dependencies: ["LeptosKitNetworkInfo", .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple-tests"
        )
    ]
)
