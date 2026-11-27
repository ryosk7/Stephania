import AVFoundation
import UIKit
import VideoToolbox
import WebRTC

/// Camera capture for the Stephania sender (docs/MASTER-ARCHITECTURE.md §12):
/// one or two built-in wide cameras, both at once through
/// AVCaptureMultiCamSession where the hardware allows it. Frames go straight
/// from the capture queue into the WebRTC video sources (CameraTrack.push);
/// nothing crosses the JS bridge.
///
/// Output is always landscape 16:9: the connection is rotated to the
/// device's landscape orientation in the ISP, so the Stephania device (which ignores
/// the RTP orientation extension) receives upright frames.
final class CameraCapture: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
  enum Failure: Error, CustomStringConvertible {
    case permission
    case noCamera(String)
    case noFormat(String, Int, Int, Int)
    case multiCamUnsupported
    case tooExpensive(Float, Float)
    case configuration(String)

    var description: String {
      switch self {
      case .permission: return "Camera access is not allowed. Enable it in Settings."
      case let .noCamera(r): return "This phone has no \(r) camera."
      case let .noFormat(r, w, h, f): return "The \(r) camera cannot capture \(w)×\(h) at \(f) fps."
      case .multiCamUnsupported: return "This phone cannot use both cameras at once."
      case let .tooExpensive(hw, pressure): return "Both cameras at this quality exceed the phone's capacity (hardware cost \(hw), pressure cost \(pressure))."
      case let .configuration(m): return "Camera setup failed: \(m)"
      }
    }
  }

  typealias Plan = TrackPlan

  /// Interruption begin/end with a reason from StephaniaCamera.types.ts InterruptionReason.
  var onInterruption: ((Bool, String) -> Void)?
  var onError: ((String) -> Void)?
  /// Capture system pressure, mapped to a ThermalState name.
  var onPressure: ((String) -> Void)?

  private var tracks: [String: CameraTrack] = [:]
  private var captureHeights: [String: Int] = [:]
  private var session: AVCaptureSession?
  private var outputs: [AVCaptureVideoDataOutput: String] = [:]
  private var devices: [String: AVCaptureDevice] = [:]
  private var observers: [NSObjectProtocol] = []
  private var pressureObservations: [NSKeyValueObservation] = []
  private let sessionQueue = DispatchQueue(label: "stephania.camera.session")
  private let frameQueue = DispatchQueue(label: "stephania.camera.frames", qos: .userInteractive)
  private let lock = NSLock()

  // MARK: capabilities

  static func device(_ role: String) -> AVCaptureDevice? {
    AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: role == "front" ? .front : .back)
  }

  static func roles() -> [String] { ["rear", "front"].filter { device($0) != nil } }

  static func hardwareH264() -> Bool {
    var list: CFArray?
    guard VTCopyVideoEncoderList(nil, &list) == noErr, let encoders = list as? [[String: Any]] else { return false }
    return encoders.contains {
      ($0[kVTVideoEncoderList_CodecType as String] as? UInt32) == kCMVideoCodecType_H264 &&
        ($0[kVTVideoEncoderList_IsHardwareAccelerated as String] as? Bool) == true
    }
  }

  /// §12 dual profiles this phone can capture concurrently: a multi-cam
  /// session is configured (not started) with the formats each profile needs
  /// and accepted only if hardwareCost and systemPressureCost stay below 1.
  static func dualProfiles() -> [String] {
    guard AVCaptureMultiCamSession.isMultiCamSupported, let rear = device("rear"), let front = device("front") else { return [] }
    let profiles: [(String, (Int, Int, Int), (Int, Int, Int))] = [
      ("720p30+720p30", (1280, 720, 30), (1280, 720, 30)),
      ("720p30+540p24", (1280, 720, 30), (960, 540, 24)),
      ("540p24+540p24", (960, 540, 24), (960, 540, 24)),
    ]
    return profiles.compactMap { name, r, f in
      guard let rf = format(for: rear, width: r.0, height: r.1, fps: r.2, multiCam: true),
            let ff = format(for: front, width: f.0, height: f.1, fps: f.2, multiCam: true) else { return nil }
      let s = AVCaptureMultiCamSession()
      s.beginConfiguration()
      defer { s.commitConfiguration() }
      for (dev, fmt, fps) in [(rear, rf, r.2), (front, ff, f.2)] {
        guard let input = try? AVCaptureDeviceInput(device: dev), s.canAddInput(input) else { return nil }
        s.addInputWithNoConnections(input)
        guard (try? configure(dev, format: fmt, fps: fps)) != nil else { return nil }
      }
      return s.hardwareCost < 1 && s.systemPressureCost < 1 ? name : nil
    }
  }

  /// Smallest format at least `width`×`height` (16:9) whose frame-rate ranges include `fps`.
  static func format(for device: AVCaptureDevice, width: Int, height: Int, fps: Int, multiCam: Bool) -> AVCaptureDevice.Format? {
    device.formats
      .filter { f in
        let d = CMVideoFormatDescriptionGetDimensions(f.formatDescription)
        return Int(d.width) >= width && Int(d.height) >= height && Int(d.width) * 9 == Int(d.height) * 16 &&
          (!multiCam || f.isMultiCamSupported) &&
          f.videoSupportedFrameRateRanges.contains { $0.minFrameRate <= Double(fps) && Double(fps) <= $0.maxFrameRate }
      }
      .min { a, b in
        let da = CMVideoFormatDescriptionGetDimensions(a.formatDescription), db = CMVideoFormatDescriptionGetDimensions(b.formatDescription)
        if da.width != db.width { return da.width < db.width }
        // Prefer binned formats (lower power) at equal size.
        return a.isVideoBinned && !b.isVideoBinned
      }
  }

  static func configure(_ device: AVCaptureDevice, format: AVCaptureDevice.Format, fps: Int) throws {
    try device.lockForConfiguration()
    defer { device.unlockForConfiguration() }
    device.activeFormat = format
    let duration = CMTime(value: 1, timescale: CMTimeScale(fps))
    device.activeVideoMinFrameDuration = duration
    device.activeVideoMaxFrameDuration = duration
  }

  // MARK: lifecycle

  static func requestPermission() async -> Bool {
    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized: return true
    case .notDetermined: return await AVCaptureDevice.requestAccess(for: .video)
    default: return false
    }
  }

  /// Opens the cameras for `plans` (one per role) and starts frames flowing into new CameraTracks.
  func start(mode: String, plans: [Plan]) async throws {
    guard await Self.requestPermission() else { throw Failure.permission }
    try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
      sessionQueue.async {
        do { try self.configureAndRun(mode: mode, plans: plans); c.resume() } catch { c.resume(throwing: error) }
      }
    }
  }

  func stop() async {
    await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
      sessionQueue.async {
        self.teardown()
        c.resume()
      }
    }
  }

  func track(_ role: String) -> CameraTrack? { lock.withLock { tracks[role] } }

  var heights: [String: Int] { lock.withLock { captureHeights } }
  var allTracks: [CameraTrack] { lock.withLock { Array(tracks.values) } }

  private func configureAndRun(mode: String, plans: [Plan]) throws {
    teardown()
    let roles = mode == "dual" ? ["rear", "front"] : [mode]
    let dual = roles.count == 2
    if dual && !AVCaptureMultiCamSession.isMultiCamSupported { throw Failure.multiCamUnsupported }
    let s: AVCaptureSession = dual ? AVCaptureMultiCamSession() : AVCaptureSession()
    s.beginConfiguration()
    if !dual { s.sessionPreset = .inputPriority }
    var newTracks: [String: CameraTrack] = [:]
    var heights: [String: Int] = [:]
    var newOutputs: [AVCaptureVideoDataOutput: String] = [:]
    do {
      let created = SenderPeer.makeTracks(roles: roles)
      for track in created {
        let role = track.role
        guard let plan = plans.first(where: { $0.role == role }) else { throw Failure.configuration("no plan for \(role)") }
        guard let dev = Self.device(role) else { throw Failure.noCamera(role) }
        guard let fmt = Self.format(for: dev, width: plan.width, height: plan.height, fps: plan.fps, multiCam: dual) else {
          throw Failure.noFormat(role, plan.width, plan.height, plan.fps)
        }
        let input = try AVCaptureDeviceInput(device: dev)
        let output = AVCaptureVideoDataOutput()
        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange]
        output.alwaysDiscardsLateVideoFrames = true
        output.setSampleBufferDelegate(self, queue: frameQueue)
        if let multi = s as? AVCaptureMultiCamSession {
          guard multi.canAddInput(input), multi.canAddOutput(output) else { throw Failure.configuration("cannot add \(role) camera") }
          multi.addInputWithNoConnections(input)
          multi.addOutputWithNoConnections(output)
          let ports = input.ports(for: .video, sourceDeviceType: dev.deviceType, sourceDevicePosition: dev.position)
          let connection = AVCaptureConnection(inputPorts: ports, output: output)
          guard multi.canAddConnection(connection) else { throw Failure.configuration("cannot connect \(role) camera") }
          multi.addConnection(connection)
        } else {
          guard s.canAddInput(input), s.canAddOutput(output) else { throw Failure.configuration("cannot add \(role) camera") }
          s.addInput(input)
          s.addOutput(output)
        }
        // Set the format after adding the input: adding an input may reset it.
        try Self.configure(dev, format: fmt, fps: plan.fps)
        if let c = output.connection(with: .video) {
          c.isVideoMirrored = false              // the audience sees the scene as it is
          if c.isVideoStabilizationSupported { c.preferredVideoStabilizationMode = .off } // stabilisation adds latency
        }
        let dims = CMVideoFormatDescriptionGetDimensions(fmt.formatDescription)
        heights[role] = Int(dims.height)
        newOutputs[output] = role
        devices[role] = dev
        newTracks[role] = track
        track.source.adaptOutputFormat(toWidth: Int32(plan.width), height: Int32(plan.height), fps: Int32(plan.fps))
      }
      if let multi = s as? AVCaptureMultiCamSession, multi.hardwareCost >= 1 || multi.systemPressureCost >= 1 {
        throw Failure.tooExpensive(multi.hardwareCost, multi.systemPressureCost)
      }
    } catch {
      s.commitConfiguration()
      devices = [:]
      throw error
    }
    s.commitConfiguration()
    lock.lock()
    tracks = newTracks
    captureHeights = heights
    outputs = newOutputs
    lock.unlock()
    session = s
    applyRotation()
    observe(s)
    s.startRunning()
  }

  private func teardown() {
    observers.forEach(NotificationCenter.default.removeObserver)
    observers = []
    pressureObservations = []
    coordinatorObservations = []
    coordinators = [:]
    session?.stopRunning()
    session = nil
    devices = [:]
    lock.lock()
    outputs = [:]
    tracks = [:]
    captureHeights = [:]
    lock.unlock()
  }

  // MARK: notifications

  private func observe(_ s: AVCaptureSession) {
    let nc = NotificationCenter.default
    observers.append(nc.addObserver(forName: AVCaptureSession.wasInterruptedNotification, object: s, queue: nil) { [weak self] n in
      let raw = (n.userInfo?[AVCaptureSessionInterruptionReasonKey] as? Int).flatMap(AVCaptureSession.InterruptionReason.init(rawValue:))
      self?.onInterruption?(true, Self.reasonName(raw))
    })
    observers.append(nc.addObserver(forName: AVCaptureSession.interruptionEndedNotification, object: s, queue: nil) { [weak self] _ in
      self?.onInterruption?(false, "unknown")
    })
    observers.append(nc.addObserver(forName: AVCaptureSession.runtimeErrorNotification, object: s, queue: nil) { [weak self] n in
      let error = n.userInfo?[AVCaptureSessionErrorKey] as? AVError
      // Media services reset: restart the session once; report anything else.
      if error?.code == .mediaServicesWereReset {
        self?.sessionQueue.async { if self?.session?.isRunning == false { self?.session?.startRunning() } }
      } else {
        self?.onError?(error?.localizedDescription ?? "capture runtime error")
      }
    })
    if #unavailable(iOS 17.0) {
      DispatchQueue.main.async { UIDevice.current.beginGeneratingDeviceOrientationNotifications() }
      observers.append(nc.addObserver(forName: UIDevice.orientationDidChangeNotification, object: nil, queue: nil) { [weak self] _ in
        self?.sessionQueue.async { self?.applyRotation() }
      })
    }
    for dev in devices.values {
      pressureObservations.append(dev.observe(\.systemPressureState, options: [.new]) { [weak self] d, _ in
        self?.onPressure?(Self.pressureName(d.systemPressureState.level))
      })
    }
  }

  static func reasonName(_ r: AVCaptureSession.InterruptionReason?) -> String {
    switch r {
    case .videoDeviceNotAvailableInBackground: return "background"
    case .videoDeviceInUseByAnotherClient, .audioDeviceInUseByAnotherClient: return "in_use_by_other_app"
    case .videoDeviceNotAvailableWithMultipleForegroundApps: return "multitasking"
    case .videoDeviceNotAvailableDueToSystemPressure: return "pressure"
    default: return "unknown"
    }
  }

  static func pressureName(_ level: AVCaptureDevice.SystemPressureState.Level) -> String {
    switch level {
    case .nominal: return "nominal"
    case .fair: return "fair"
    case .serious: return "serious"
    default: return "critical"                 // critical and shutdown
    }
  }

  /// Keeps the output landscape and upright: the ISP rotates each
  /// connection to the horizon-level angle for the way the phone is held;
  /// portrait (and face up/down) keep the last landscape angle, so the
  /// Stephania always receives 16:9 landscape frames.
  private var coordinators: [String: AnyObject] = [:]
  private var lastLandscape: [String: CGFloat] = [:]
  private var coordinatorObservations: [NSKeyValueObservation] = []

  private func applyRotation() {
    let roleOutputs = lock.withLock { outputs }
    for (output, role) in roleOutputs {
      guard let c = output.connection(with: .video) else { continue }
      if #available(iOS 17.0, *) {
        let coordinator: AVCaptureDevice.RotationCoordinator
        if let existing = coordinators[role] as? AVCaptureDevice.RotationCoordinator {
          coordinator = existing
        } else {
          guard let dev = devices[role] else { continue }
          coordinator = AVCaptureDevice.RotationCoordinator(device: dev, previewLayer: nil)
          coordinators[role] = coordinator
          coordinatorObservations.append(coordinator.observe(\.videoRotationAngleForHorizonLevelCapture, options: [.new]) { [weak self] _, _ in
            self?.sessionQueue.async { self?.applyRotation() }
          })
        }
        let angle = coordinator.videoRotationAngleForHorizonLevelCapture
        if angle == 0 || angle == 180 { lastLandscape[role] = angle }
        let target = lastLandscape[role] ?? 0
        if c.isVideoRotationAngleSupported(target) { c.videoRotationAngle = target }
      } else if c.isVideoOrientationSupported {
        switch UIDevice.current.orientation {
        case .landscapeLeft: c.videoOrientation = .landscapeRight
        case .landscapeRight: c.videoOrientation = .landscapeLeft
        default: if c.videoOrientation != .landscapeLeft && c.videoOrientation != .landscapeRight { c.videoOrientation = .landscapeRight }
        }
      }
    }
  }

  // MARK: frames

  func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
    guard let video = output as? AVCaptureVideoDataOutput,
          let track = lock.withLock({ outputs[video].flatMap { tracks[$0] } }),
          let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
    let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
    let ns = Int64(CMTimeGetSeconds(pts) * 1_000_000_000)
    track.push(pixelBuffer, rotation: ._0, timestampNs: ns)
  }
}
