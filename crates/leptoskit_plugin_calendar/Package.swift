// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitCalendar",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitCalendar", targets: ["LeptosKitCalendar"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitCalendar",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitCalendarTests",
            dependencies: ["LeptosKitCalendar"],
            path: "apple-tests"
        )
    ]
)
