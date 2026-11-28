import CoreVideo
import Foundation
import WebRTC

/// Encoding target for one track (mirror of `TrackPlan` in StephaniaCamera.types.ts).
public struct TrackPlan: Equatable {
  public var role: String
  public var active: Bool
  public var width: Int
  public var height: Int
  public var fps: Int
  public var maxBitrateKbps: Int

  public init(role: String, active: Bool, width: Int, height: Int, fps: Int, maxBitrateKbps: Int) {
    (self.role, self.active, self.width, self.height, self.fps, self.maxBitrateKbps) = (role, active, width, height, fps, maxBitrateKbps)
  }
}

public enum SenderError: Error, CustomStringConvertible {
  case peerConnection
  case transceiver(String)
  case offer(String)
  case gatherTimeout
  case answer(String)
  case unknownRole(String)

  public var description: String {
    switch self {
    case .peerConnection: return "could not create the peer connection"
    case let .transceiver(r): return "could not add the \(r) camera track"
    case let .offer(m): return "offer failed: \(m)"
    case .gatherTimeout: return "ICE gathering did not complete"
    case let .answer(m): return "the answer was not accepted: \(m)"
    case let .unknownRole(r): return "no \(r) track in this session"
    }
  }
}

/// One camera track: its source (frames go in here) and the local track
/// (previews attach here).
public final class CameraTrack {
  public let role: String
  public let source: RTCVideoSource
  public let track: RTCVideoTrack
  let capturer: RTCVideoCapturer

  init(role: String, factory: RTCPeerConnectionFactory) {
    self.role = role
    source = factory.videoSource()
    // Track ids end up in a=msid; stephania-phone maps them to rear/front.
    track = factory.videoTrack(with: source, trackId: "\(role)-\(UUID().uuidString.prefix(8).lowercased())")
    capturer = RTCVideoCapturer(delegate: source)
  }

  /// Delivers one captured frame to the encoder. Called on the capture queue.
  public func push(_ pixelBuffer: CVPixelBuffer, rotation: RTCVideoRotation, timestampNs: Int64) {
    let frame = RTCVideoFrame(buffer: RTCCVPixelBuffer(pixelBuffer: pixelBuffer), rotation: rotation, timeStampNs: timestampNs)
    source.capturer(capturer, didCapture: frame)
  }
}

/// WebRTC sender for one Stephania camera session: one sendonly H.264 track per
/// camera, LAN host candidates only, non-trickle offer. Hardware encoding:
/// RTCDefaultVideoEncoderFactory uses VideoToolbox for H.264.
public final class SenderPeer: NSObject, RTCPeerConnectionDelegate {
  public static let factory: RTCPeerConnectionFactory = {
    RTCInitializeSSL()
    return RTCPeerConnectionFactory(encoderFactory: RTCDefaultVideoEncoderFactory(), decoderFactory: RTCDefaultVideoDecoderFactory())
  }()

  public static let gatherTimeout: TimeInterval = 5

  public let tracks: [CameraTrack]
  /// Peer connection state changes: new, connecting, connected, disconnected, failed, closed.
  public var onState: ((String) -> Void)?

  private var pc: RTCPeerConnection!
  private var senders: [String: RTCRtpSender] = [:]
  /// Capture size per role: plans scale down from it.
  private var captureHeight: [String: Int] = [:]
  private let lock = NSLock()
  private var gatherWaiters: [() -> Void] = []
  private var gathered = false

  /// - Parameters:
  ///   - tracks: camera tracks (sources) to send, created with `makeTracks`.
  ///   - plans: initial encoding per role; `captureHeights` the size frames arrive in.
  public init(tracks: [CameraTrack], plans: [TrackPlan], captureHeights: [String: Int]) throws {
    self.tracks = tracks
    super.init()
    captureHeight = captureHeights
    let config = RTCConfiguration()
    config.sdpSemantics = .unifiedPlan
    config.iceServers = []                       // LAN only: no STUN/TURN
    config.bundlePolicy = .maxBundle
    config.rtcpMuxPolicy = .require
    config.continualGatheringPolicy = .gatherOnce
    config.tcpCandidatePolicy = .disabled
    let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)
    guard let pc = Self.factory.peerConnection(with: config, constraints: constraints, delegate: self) else { throw SenderError.peerConnection }
    self.pc = pc
    for t in tracks {
      let initObj = RTCRtpTransceiverInit()
      initObj.direction = .sendOnly
      initObj.streamIds = ["stephania"]
      if let plan = plans.first(where: { $0.role == t.role }) {
        let e = RTCRtpEncodingParameters()
        apply(plan, to: e, role: t.role)
        initObj.sendEncodings = [e]
      }
      guard let tr = pc.addTransceiver(with: t.track, init: initObj) else { throw SenderError.transceiver(t.role) }
      // Broadcast sources keep their geometry: under bandwidth pressure
      // lower the frame rate and quality, never silently the resolution
      // (libwebrtc's default starts low and scales up). Explicit plans
      // (scaleResolutionDownBy) still change it.
      let params = tr.sender.parameters
      params.degradationPreference = NSNumber(value: RTCDegradationPreference.maintainResolution.rawValue)
      tr.sender.parameters = params
      senders[t.role] = tr.sender
    }
  }

  public static func makeTracks(roles: [String]) -> [CameraTrack] {
    roles.map { CameraTrack(role: $0, factory: factory) }
  }

  /// MediaStreamTrack id → role, as the offer carries them.
  public var trackMap: [String: String] {
    Dictionary(uniqueKeysWithValues: tracks.map { ($0.track.trackId, $0.role) })
  }

  /// Creates the offer restricted to baseline H.264, sets it locally and
  /// returns it once ICE gathering is complete (candidates inline).
  public func makeOffer() async throws -> String {
    let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)
    let raw: RTCSessionDescription = try await withCheckedThrowingContinuation { c in
      pc.offer(for: constraints) { sdp, error in
        if let sdp { c.resume(returning: sdp) } else { c.resume(throwing: SenderError.offer(error?.localizedDescription ?? "no SDP")) }
      }
    }
    let munged = try SdpMunger.restrictToBaselineH264(raw.sdp)
    try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
      pc.setLocalDescription(RTCSessionDescription(type: .offer, sdp: munged)) { error in
        if let error { c.resume(throwing: SenderError.offer(error.localizedDescription)) } else { c.resume() }
      }
    }
    try await waitForGathering()
    guard let sdp = pc.localDescription?.sdp else { throw SenderError.offer("no local description") }
    return sdp
  }

  public func setAnswer(_ sdp: String) async throws {
    try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
      pc.setRemoteDescription(RTCSessionDescription(type: .answer, sdp: sdp)) { error in
        if let error { c.resume(throwing: SenderError.answer(error.localizedDescription)) } else { c.resume() }
      }
    }
  }

  /// Applies bitrate/fps/resolution/active per role without renegotiation.
  public func apply(_ plans: [TrackPlan]) throws {
    for plan in plans {
      guard let sender = senders[plan.role] else { throw SenderError.unknownRole(plan.role) }
      let params = sender.parameters
      guard let e = params.encodings.first else { continue }
      apply(plan, to: e, role: plan.role)
      sender.parameters = params
    }
  }

  /// Current encoding of a role (tests, diagnostics).
  public func encoding(_ role: String) -> RTCRtpEncodingParameters? { senders[role]?.parameters.encodings.first }

  /// Encoder-side statistics per track (outbound-rtp): frames encoded and
  /// sent, size, encoder implementation, quality limitation. For
  /// diagnostics and tests; values are as libwebrtc reports them.
  public func outboundStats() async -> [[String: String]] {
    await withCheckedContinuation { c in
      pc.statistics { report in
        let keys = ["mid", "framesEncoded", "framesSent", "frameWidth", "frameHeight", "encoderImplementation", "qualityLimitationReason", "bytesSent", "active"]
        let rows = report.statistics.values.filter { $0.type == "outbound-rtp" }.map { s in
          Dictionary(uniqueKeysWithValues: keys.compactMap { k in s.values[k].map { (k, "\($0)") } })
        }
        c.resume(returning: rows)
      }
    }
  }

  /// ICE/DTLS state for diagnostics.
  public var connectionState: String { "\(pc.connectionState.rawValue)/ice \(pc.iceConnectionState.rawValue)" }

  public func close() {
    pc?.close()
  }

  private func apply(_ plan: TrackPlan, to e: RTCRtpEncodingParameters, role: String) {
    e.isActive = plan.active
    e.maxBitrateBps = NSNumber(value: plan.maxBitrateKbps * 1000)
    e.maxFramerate = NSNumber(value: plan.fps)
    let from = captureHeight[role] ?? plan.height
    e.scaleResolutionDownBy = NSNumber(value: max(1.0, Double(from) / Double(max(plan.height, 1))))
  }

  /// Returns once ICE gathering is complete, or throws `gatherTimeout`.
  /// One continuation, resumed by whichever of the gathering callback and
  /// the timer comes first: a task-group timeout cannot work here, because
  /// the group would still wait for a continuation that ignores
  /// cancellation (and that never resumes for a closed peer connection).
  func waitForGathering(timeout: TimeInterval = SenderPeer.gatherTimeout) async throws {
    try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
      let once = ResumeOnce(c)
      lock.lock()
      if gathered { lock.unlock(); once.resume(.success(())); return }
      gatherWaiters.append { once.resume(.success(())) }
      lock.unlock()
      DispatchQueue.global().asyncAfter(deadline: .now() + timeout) { once.resume(.failure(SenderError.gatherTimeout)) }
    }
  }

  // MARK: RTCPeerConnectionDelegate

  public func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {
    guard newState == .complete else { return }
    lock.lock()
    gathered = true
    let waiters = gatherWaiters
    gatherWaiters = []
    lock.unlock()
    waiters.forEach { $0() }
  }

  public func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCPeerConnectionState) {
    let name: String
    switch newState {
    case .new: name = "new"
    case .connecting: name = "connecting"
    case .connected: name = "connected"
    case .disconnected: name = "disconnected"
    case .failed: name = "failed"
    case .closed: name = "closed"
    @unknown default: return
    }
    onState?(name)
  }

  public func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
  public func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}
  public func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}
  public func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {}
  public func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {}
  public func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {}
  public func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}
  public func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {}
}

/// Resumes a continuation at most once; later calls are ignored.
private final class ResumeOnce: @unchecked Sendable {
  private let lock = NSLock()
  private var continuation: CheckedContinuation<Void, Error>?

  init(_ continuation: CheckedContinuation<Void, Error>) { self.continuation = continuation }

  func resume(_ result: Result<Void, Error>) {
    lock.lock()
    let c = continuation
    continuation = nil
    lock.unlock()
    c?.resume(with: result)
  }
}
