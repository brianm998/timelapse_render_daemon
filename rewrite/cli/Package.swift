// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "timelapse-render-cli",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .executable(name: "timelapse-render", targets: ["timelapse-render-cli"]),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-argument-parser", from: "1.3.0"),
        .package(name: "TimelapseRenderCore", path: "../TimelapseRenderCore"),
    ],
    targets: [
        .executableTarget(
            name: "timelapse-render-cli",
            dependencies: [
                .product(name: "ArgumentParser", package: "swift-argument-parser"),
                .product(name: "TimelapseRenderCore", package: "TimelapseRenderCore"),
            ],
            path: "Sources/timelapse-render-cli"
        ),
    ]
)
