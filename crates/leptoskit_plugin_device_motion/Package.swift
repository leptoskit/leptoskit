// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitDeviceMotion",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitDeviceMotion", targets: ["LeptosKitDeviceMotion"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitDeviceMotion",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitDeviceMotionTests",
            dependencies: ["LeptosKitDeviceMotion"],
            path: "apple-tests"
        )
    ]
)
