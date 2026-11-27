// swift-tools-version:5.9
// Platform-neutral core of the StephaniaCamera module: WebRTC peer, H.264
// codec restriction and encoding plans. No AVFoundation capture and no
// Expo here, so `swift test` on macOS exercises the real libwebrtc sender
// with synthetic frames (ci: mobile-ios-camera).
import PackageDescription

let package = Package(
  name: "CameraSender",
  platforms: [.macOS(.v13), .iOS(.v16)],
  products: [.library(name: "CameraSender", targets: ["CameraSender"])],
  dependencies: [
    // Same build as the app's pod (StephaniaCamera.podspec: WebRTC-SDK).
    .package(url: "https://github.com/webrtc-sdk/Specs.git", exact: "150.7871.09"),
  ],
  targets: [
    .target(name: "CameraSender", dependencies: [.product(name: "WebRTC", package: "Specs")]),
    .testTarget(name: "CameraSenderTests", dependencies: ["CameraSender"]),
  ]
)
