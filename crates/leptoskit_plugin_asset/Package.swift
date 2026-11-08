// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitAsset",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitAsset", targets: ["LeptosKitAsset"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(name: "LeptosKitAsset", dependencies: [
            .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
        ], path: "apple"),
        .testTarget(name: "AssetTests", dependencies: ["LeptosKitAsset"], path: "apple-tests")
    ]
)
