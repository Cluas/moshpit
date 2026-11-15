// swift-tools-version: 5.9
import PackageDescription
let package = Package(
    name: "wkbench",
    platforms: [.macOS("14.0")],
    dependencies: [.package(url: "https://github.com/argmaxinc/WhisperKit.git", exact: "1.1.0")],
    targets: [.executableTarget(name: "wkbench", dependencies: [.product(name: "WhisperKit", package: "WhisperKit")])]
)
