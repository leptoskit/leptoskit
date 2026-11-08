// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitLivePhoto",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitLivePhoto", targets: ["LeptosKitLivePhoto"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitLivePhoto",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitLivePhotoTests",
            dependencies: ["LeptosKitLivePhoto"],
            path: "apple-tests"
        )
    ]
)
