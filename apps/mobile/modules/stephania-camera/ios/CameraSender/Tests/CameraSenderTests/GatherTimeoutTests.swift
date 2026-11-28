import Foundation
import XCTest
@testable import CameraSender

/// The ICE gather timeout must fire even when gathering never completes
/// (here: no offer was made, so it never starts).
final class GatherTimeoutTests: XCTestCase {
  func testGatherWaitTimesOutWhenGatheringNeverCompletes() async throws {
    let sender = try SenderPeer(tracks: SenderPeer.makeTracks(roles: ["rear"]), plans: [], captureHeights: [:])
    defer { sender.close() }
    let started = Date()
    do {
      try await sender.waitForGathering(timeout: 0.3)
      XCTFail("gathering cannot have completed")
    } catch SenderError.gatherTimeout {
      XCTAssertLessThan(Date().timeIntervalSince(started), 3, "timeout fires on time")
    }
  }

  func testGatherWaitTimesOutAfterThePeerIsClosed() async throws {
    let sender = try SenderPeer(tracks: SenderPeer.makeTracks(roles: ["rear"]), plans: [], captureHeights: [:])
    sender.close()
    do {
      try await sender.waitForGathering(timeout: 0.3)
      XCTFail("a closed peer never completes gathering")
    } catch SenderError.gatherTimeout {}
  }
}
