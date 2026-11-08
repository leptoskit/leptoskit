// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitContacts",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitContacts", targets: ["LeptosKitContacts"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitContacts",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitContactsTests",
            dependencies: ["LeptosKitContacts"],
            path: "apple-tests"
        )
    ]
)
