// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitWatchSync",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitWatchSync", targets: ["LeptosKitWatchSync"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitWatchSync",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
