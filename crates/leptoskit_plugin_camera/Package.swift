// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitCamera",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitCamera", targets: ["LeptosKitCamera"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitCamera",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitCameraTests",
            dependencies: [
                "LeptosKitCamera",
                .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
            ],
            path: "apple-tests"
        )
    ]
)
