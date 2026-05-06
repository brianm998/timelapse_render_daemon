// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "TimelapseRenderCore",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .library(name: "TimelapseRenderCore", targets: ["TimelapseRenderCore"]),
    ],
    dependencies: [],
    targets: [
        .target(
            name: "TimelapseRenderCore",
            path: "Sources/TimelapseRenderCore"
        ),
        .testTarget(
            name: "TimelapseRenderCoreTests",
            dependencies: ["TimelapseRenderCore"],
            path: "Tests/TimelapseRenderCoreTests"
        ),
    ]
)
