// swift-tools-version: 5.9
import PackageDescription

let package = Package(
	name: "LeptoskitPod",
	platforms: [.macOS(.v12), .iOS(.v15)],
	products: [
		.library(name: "LeptoskitPod", targets: ["LeptoskitPod"]),
	],
	targets: [
		.target(name: "LeptoskitPod"),
		.testTarget(name: "LeptoskitPodTests", dependencies: ["LeptoskitPod"]),
	]
)
