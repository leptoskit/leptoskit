// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitWebBrowser",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitWebBrowser", targets: ["LeptosKitWebBrowser"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitWebBrowser",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitWebBrowserTests",
            dependencies: ["LeptosKitWebBrowser"],
            path: "apple-tests"
        )
    ]
)
