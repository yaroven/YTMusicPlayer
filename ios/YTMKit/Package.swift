// swift-tools-version:6.0
// YTMKit: the iPhone app's core — YouTube Music catalog, stream URLs,
// lyrics — with no UI, so it builds and self-tests on macOS too.
import PackageDescription

let package = Package(
    name: "YTMKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "YTMKit", targets: ["YTMKit"]),
    ],
    targets: [
        .target(name: "YTMKit"),
        // `swift run YTMKitChecks [--live]`: offline parsing checks, and the
        // live ones against YouTube (no XCTest in Command Line Tools).
        .executableTarget(name: "YTMKitChecks", dependencies: ["YTMKit"]),
    ]
)
