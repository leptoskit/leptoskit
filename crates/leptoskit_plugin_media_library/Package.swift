// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitMediaLibrary",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitMediaLibrary", targets: ["LeptosKitMediaLibrary"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitMediaLibrary",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitMediaLibraryTests",
            dependencies: ["LeptosKitMediaLibrary"],
            path: "apple-tests"
        )
    ]
)
