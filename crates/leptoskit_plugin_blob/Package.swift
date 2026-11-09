// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitBlob",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitBlob", targets: ["LeptosKitBlob"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitBlob",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "BlobTests",
            dependencies: ["LeptosKitBlob"],
            path: "apple-tests"
        )
    ]
)
