// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitVideoThumbnails",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitVideoThumbnails", targets: ["LeptosKitVideoThumbnails"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitVideoThumbnails",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitVideoThumbnailsTests",
            dependencies: ["LeptosKitVideoThumbnails"],
            path: "apple-tests"
        )
    ]
)
