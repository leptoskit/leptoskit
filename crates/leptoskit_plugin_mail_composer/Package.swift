// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitMailComposer",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitMailComposer", targets: ["LeptosKitMailComposer"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitMailComposer",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitMailComposerTests",
            dependencies: ["LeptosKitMailComposer"],
            path: "apple-tests"
        )
    ]
)
