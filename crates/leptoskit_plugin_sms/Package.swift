// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitSms",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitSms", targets: ["LeptosKitSms"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitSms",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitSmsTests",
            dependencies: ["LeptosKitSms"],
            path: "apple-tests"
        )
    ]
)
