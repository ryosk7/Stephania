// swift-tools-version:5.9
// Pure trust logic of the T3BoxPinnedFetch module, testable with `swift test`
// on macOS without building the app (ci: mobile-ios-trust).
import PackageDescription

let package = Package(
  name: "PinnedTrust",
  platforms: [.macOS(.v13), .iOS(.v16)],
  products: [.library(name: "PinnedTrust", targets: ["PinnedTrust"])],
  targets: [
    .target(name: "PinnedTrust"),
    .testTarget(name: "PinnedTrustTests", dependencies: ["PinnedTrust"], resources: [.copy("Fixtures")]),
  ]
)
