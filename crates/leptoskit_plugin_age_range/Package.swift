// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitAgeRange",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitAgeRange", targets: ["LeptosKitAgeRange"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(name: "LeptosKitAgeRange", dependencies: [
            .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
        ], path: "apple"),
        .testTarget(
            name: "AgeRangeTests",
            dependencies: ["LeptosKitAgeRange"],
            path: "apple-tests"
        )
    ]
)
