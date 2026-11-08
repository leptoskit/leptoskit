// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitVideo",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitVideo", targets: ["LeptosKitVideo"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitVideo",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitVideoTests",
            dependencies: [
                "LeptosKitVideo",
                .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
            ],
            path: "apple-tests"
        )
    ]
)
