// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitPrint",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitPrint", targets: ["LeptosKitPrint"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitPrint",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitPrintTests",
            dependencies: ["LeptosKitPrint"],
            path: "apple-tests"
        )
    ]
)
