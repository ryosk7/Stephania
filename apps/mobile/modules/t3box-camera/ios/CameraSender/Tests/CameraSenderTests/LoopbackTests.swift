import CoreVideo
import Foundation
import WebRTC
import XCTest
@testable import CameraSender

/// Real libwebrtc on both ends, in one process: the T3Box sender with
/// synthetic frames of different brightness per camera, and a receiver that
/// (like t3box-phone) only knows H.264 constrained baseline. Checks the
/// negotiated codec, that both tracks decode with the right content (not
/// swapped), and that plans change the running encoders.
final class LoopbackTests: XCTestCase {
  func testDualTracksArriveAsBaselineH264WithTheirOwnContent() async throws {
    let tracks = SenderPeer.makeTracks(roles: ["rear", "front"])
    let plans = [
      TrackPlan(role: "rear", active: true, width: 1280, height: 720, fps: 30, maxBitrateKbps: 2500),
      TrackPlan(role: "front", active: true, width: 1280, height: 720, fps: 30, maxBitrateKbps: 2500),
    ]
    let sender = try SenderPeer(tracks: tracks, plans: plans, captureHeights: ["rear": 720, "front": 720])
    defer { sender.close() }
    let offer = try await sender.makeOffer()

    // Offer contract (src/camera/sdp.ts checks the same on the JS side).
    let videoLines = offer.components(separatedBy: "\r\n").filter { $0.hasPrefix("m=video") }
    XCTAssertEqual(videoLines.count, 2)
    let fmtp = offer.components(separatedBy: "\r\n").filter { $0.hasPrefix("a=fmtp:") && $0.contains("profile-level-id") }
    print("offer H.264:", fmtp)
    XCTAssertFalse(fmtp.isEmpty)
    XCTAssertFalse(offer.contains("VP8/90000"))
    XCTAssertTrue(offer.contains("a=candidate:"), "non-trickle offer must carry candidates")
    for (id, _) in sender.trackMap { XCTAssertTrue(offer.contains(" \(id)\r\n"), "a=msid for \(id)") }
    if let path = ProcessInfo.processInfo.environment["T3BOX_OFFER_OUT"] {
      // Real libwebrtc offer for the Linux job that feeds it to t3box-phone.
      let json = try JSONSerialization.data(withJSONObject: ["sdp": offer, "tracks": sender.trackMap], options: [.sortedKeys])
      try json.write(to: URL(fileURLWithPath: path))
    }

    let receiver = Receiver()
    defer { receiver.close() }
    let answer = try await receiver.answer(offer)
    XCTAssertTrue(answer.contains("H264/90000"))
    try await sender.setAnswer(answer)

    // 4 s of frames: rear dark, front bright.
    let feeder = Feeder(tracks: tracks, luma: ["rear": 50, "front": 200])
    feeder.start()
    defer { feeder.stop() }
    try await waitUntil(10) { receiver.frames(mid: "0") >= 30 && receiver.frames(mid: "1") >= 30 }
    XCTAssertEqual(sender.trackMap.values.sorted(), ["front", "rear"])
    XCTAssertEqual(receiver.size(mid: "0")?.height, 720)
    XCTAssertLessThan(receiver.meanLuma(mid: "0"), 90, "rear (mid 0) must carry the dark frames")
    XCTAssertGreaterThan(receiver.meanLuma(mid: "1"), 160, "front (mid 1) must carry the bright frames")

    // Thermal step: front off, rear down to 540p. Front frames stop, rear shrinks.
    try sender.apply([
      TrackPlan(role: "rear", active: true, width: 960, height: 540, fps: 24, maxBitrateKbps: 1200),
      TrackPlan(role: "front", active: false, width: 960, height: 540, fps: 24, maxBitrateKbps: 1200),
    ])
    XCTAssertEqual(sender.encoding("front")?.isActive, false)
    XCTAssertEqual(sender.encoding("rear")?.maxBitrateBps, 1_200_000)
    try await waitUntil(10) { receiver.size(mid: "0")?.height == 540 }
    try await Task.sleep(nanoseconds: 1_000_000_000)
    let frozen = receiver.frames(mid: "1")
    try await Task.sleep(nanoseconds: 1_500_000_000)
    XCTAssertLessThanOrEqual(receiver.frames(mid: "1") - frozen, 1, "inactive front track must stop sending")
    let rearBefore = receiver.frames(mid: "0")
    try await Task.sleep(nanoseconds: 1_000_000_000)
    XCTAssertGreaterThan(receiver.frames(mid: "0") - rearBefore, 10, "rear keeps flowing")

    // And back on.
    try sender.apply(plans)
    let resumed = receiver.frames(mid: "1")
    try await waitUntil(10) { receiver.frames(mid: "1") > resumed + 10 }
  }

  func testSingleCameraOffer() async throws {
    let tracks = SenderPeer.makeTracks(roles: ["rear"])
    let sender = try SenderPeer(tracks: tracks, plans: [TrackPlan(role: "rear", active: true, width: 1280, height: 720, fps: 30, maxBitrateKbps: 2500)], captureHeights: ["rear": 720])
    defer { sender.close() }
    let offer = try await sender.makeOffer()
    XCTAssertEqual(offer.components(separatedBy: "\r\n").filter { $0.hasPrefix("m=video") }.count, 1)
    XCTAssertThrowsError(try sender.apply([TrackPlan(role: "front", active: true, width: 1, height: 1, fps: 1, maxBitrateKbps: 1)]))
  }
}

struct Timeout: Error, CustomStringConvertible {
  let seconds: Double
  let line: UInt
  var description: String { "condition not met within \(seconds) s (line \(line))" }
}

/// Polls `condition`; a timeout is a test failure, never a skip.
func waitUntil(_ seconds: Double, line: UInt = #line, _ condition: @escaping () -> Bool) async throws {
  let deadline = Date().addingTimeInterval(seconds)
  while !condition() {
    if Date() > deadline { throw Timeout(seconds: seconds, line: line) }
    try await Task.sleep(nanoseconds: 50_000_000)
  }
}

/// Pushes solid-luma NV12 frames at 30 fps, like the capture output would.
final class Feeder {
  let tracks: [CameraTrack]
  let luma: [String: UInt8]
  private var timer: DispatchSourceTimer?
  private var buffers: [String: CVPixelBuffer] = [:]
  private let start0 = DispatchTime.now().uptimeNanoseconds

  init(tracks: [CameraTrack], luma: [String: UInt8]) {
    self.tracks = tracks
    self.luma = luma
    for t in tracks { buffers[t.role] = Self.frame(width: 1280, height: 720, y: luma[t.role] ?? 128) }
  }

  func start() {
    let t = DispatchSource.makeTimerSource(queue: DispatchQueue(label: "feeder"))
    t.schedule(deadline: .now(), repeating: .milliseconds(33))
    t.setEventHandler { [self] in
      let ts = Int64(DispatchTime.now().uptimeNanoseconds - start0)
      for track in tracks { track.push(buffers[track.role]!, rotation: ._0, timestampNs: ts) }
    }
    t.resume()
    timer = t
  }

  func stop() { timer?.cancel() }

  static func frame(width: Int, height: Int, y: UInt8) -> CVPixelBuffer {
    var pb: CVPixelBuffer?
    let attrs = [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary
    CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, attrs, &pb)
    let buffer = pb!
    CVPixelBufferLockBaseAddress(buffer, [])
    for plane in 0..<2 {
      let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane)!
      memset(base, Int32(plane == 0 ? y : 128), CVPixelBufferGetBytesPerRowOfPlane(buffer, plane) * CVPixelBufferGetHeightOfPlane(buffer, plane))
    }
    CVPixelBufferUnlockBaseAddress(buffer, [])
    return buffer
  }
}

/// Receiving end restricted like t3box-phone: H.264 only, answers non-trickle.
final class Receiver: NSObject, RTCPeerConnectionDelegate {
  private var pc: RTCPeerConnection!
  private var gathered = false
  private var renderers: [String: CountingRenderer] = [:]
  private let lock = NSLock()

  override init() {
    super.init()
    let config = RTCConfiguration()
    config.sdpSemantics = .unifiedPlan
    config.bundlePolicy = .maxBundle
    config.rtcpMuxPolicy = .require
    pc = SenderPeer.factory.peerConnection(with: config, constraints: RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil), delegate: self)
  }

  func answer(_ offer: String) async throws -> String {
    try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
      pc.setRemoteDescription(RTCSessionDescription(type: .offer, sdp: offer)) { e in e.map { c.resume(throwing: $0) } ?? c.resume() }
    }
    for tr in pc.transceivers {
      guard let track = tr.receiver.track as? RTCVideoTrack else { continue }
      let mid = tr.mid
      let r = CountingRenderer()
      lock.lock(); renderers[mid] = r; lock.unlock()
      track.add(r)
    }
    let answer: RTCSessionDescription = try await withCheckedThrowingContinuation { c in
      pc.answer(for: RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)) { sdp, e in
        if let sdp { c.resume(returning: sdp) } else { c.resume(throwing: e ?? NSError(domain: "t3box", code: 2)) }
      }
    }
    // Like t3box-phone: answer with H.264 only.
    let restricted = try SdpMunger.restrictToBaselineH264(answer.sdp)
    try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
      pc.setLocalDescription(RTCSessionDescription(type: .answer, sdp: restricted)) { e in e.map { c.resume(throwing: $0) } ?? c.resume() }
    }
    try await waitUntil(5) { [self] in lock.lock(); defer { lock.unlock() }; return gathered }
    return pc.localDescription!.sdp
  }

  func frames(mid: String) -> Int { renderer(mid)?.count ?? 0 }
  func size(mid: String) -> CGSize? { renderer(mid)?.size }
  func meanLuma(mid: String) -> Double { renderer(mid)?.luma ?? -1 }
  func close() { pc.close() }

  private func renderer(_ mid: String) -> CountingRenderer? { lock.lock(); defer { lock.unlock() }; return renderers[mid] }

  func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {
    if newState == .complete { lock.lock(); gathered = true; lock.unlock() }
  }
  func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
  func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}
  func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}
  func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {}
  func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {}
  func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {}
  func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}
  func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {}
}

/// Counts decoded frames and samples their size and mean luma.
final class CountingRenderer: NSObject, RTCVideoRenderer {
  private let lock = NSLock()
  private var _count = 0
  private var _size = CGSize.zero
  private var _luma = -1.0

  var count: Int { lock.lock(); defer { lock.unlock() }; return _count }
  var size: CGSize? { lock.lock(); defer { lock.unlock() }; return _count > 0 ? _size : nil }
  var luma: Double { lock.lock(); defer { lock.unlock() }; return _luma }

  func setSize(_ size: CGSize) {}

  func renderFrame(_ frame: RTCVideoFrame?) {
    guard let frame else { return }
    let i420 = frame.buffer.toI420()
    var sum = 0
    var n = 0
    // Sample the centre rows of the luma plane.
    let y = i420.dataY
    for row in stride(from: Int(i420.height) / 4, to: Int(i420.height) * 3 / 4, by: 8) {
      for col in stride(from: Int(i420.width) / 4, to: Int(i420.width) * 3 / 4, by: 8) {
        sum += Int(y[row * Int(i420.strideY) + col])
        n += 1
      }
    }
    lock.lock()
    _count += 1
    _size = CGSize(width: Int(frame.width), height: Int(frame.height))
    _luma = n > 0 ? Double(sum) / Double(n) : -1
    lock.unlock()
  }
}
