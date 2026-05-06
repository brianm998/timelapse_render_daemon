// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "TimelapseRenderGUI",
    platforms: [
        .macOS(.v14),
    ],
    products: [
        .executable(name: "TimelapseRenderGUI", targets: ["TimelapseRenderGUI"]),
    ],
    dependencies: [
        .package(name: "TimelapseRenderCore", path: "../TimelapseRenderCore"),
    ],
    targets: [
        .executableTarget(
            name: "TimelapseRenderGUI",
            dependencies: [
                .product(name: "TimelapseRenderCore", package: "TimelapseRenderCore"),
            ],
            path: "Sources/TimelapseRenderGUI"
        ),
    ]
)
