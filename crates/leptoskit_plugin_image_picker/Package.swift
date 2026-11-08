// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitImagePicker",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitImagePicker", targets: ["LeptosKitImagePicker"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitImagePicker",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitImagePickerTests",
            dependencies: ["LeptosKitImagePicker", .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple-tests"
        )
    ]
)
