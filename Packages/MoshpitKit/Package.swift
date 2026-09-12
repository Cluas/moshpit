// swift-tools-version: 6.0
// The code every Moshpit process shares. The app, the widget extension
// (MoshpitIsland), the notification service extension (MoshpitPush) and the
// share extension (MoshpitShare) link this one library instead of compiling
// the same files four times by path, so the line between "app" and "shared"
// is a module boundary the compiler checks: anything an extension needs has
// to be `public` here, and nothing here can reach back into the app.
//
// Language mode and platform mirror project.yml (SWIFT_VERSION 5.9, iOS 18).
import PackageDescription

let package = Package(
    name: "MoshpitKit",
    platforms: [.iOS(.v18)],
    products: [
        .library(name: "MoshpitKit", targets: ["MoshpitKit"])
    ],
    targets: [
        .target(name: "MoshpitKit")
    ],
    swiftLanguageModes: [.v5]
)
