// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitCellular",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitCellular", targets: ["LeptosKitCellular"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitCellular",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitCellularTests",
            dependencies: ["LeptosKitCellular", .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple-tests"
        )
    ]
)
