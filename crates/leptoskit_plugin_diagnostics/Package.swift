// swift-tools-version:6.0
import PackageDescription
let package = Package(
    name: "LeptosKitDiagnostics",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitDiagnostics", targets: ["LeptosKitDiagnostics"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(name: "LeptosKitDiagnostics", dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")], path: "apple"),
        .testTarget(name: "DiagnosticsTests", dependencies: ["LeptosKitDiagnostics"], path: "apple-tests")
    ]
)
