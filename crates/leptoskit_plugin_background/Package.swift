// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitBackground",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitBackground", targets: ["LeptosKitBackground"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitBackground",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitBackgroundTests",
            dependencies: ["LeptosKitBackground"],
            path: "apple-tests"
        )
    ]
)
