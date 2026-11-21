// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitWearable",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitWearable", targets: ["LeptosKitWearable"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitWearable",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
