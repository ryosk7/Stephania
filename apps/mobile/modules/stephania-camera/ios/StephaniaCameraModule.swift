import ExpoModulesCore
import Foundation
import WebRTC

/// Encoding plan from JS (StephaniaCamera.types.ts TrackPlan).
struct TrackPlanRecord: Record {
  @Field var role: String = "rear"
  @Field var active: Bool = true
  @Field var width: Int = 1280
  @Field var height: Int = 720
  @Field var fps: Int = 30
  @Field var maxBitrateKbps: Int = 2500

  var plan: TrackPlan {
    TrackPlan(role: role, active: active, width: width, height: height, fps: fps, maxBitrateKbps: maxBitrateKbps)
  }
}

/// Expo binding of the camera sender. Implements the platform contract in
/// src/StephaniaCamera.types.ts: capture, encode and transport only; every
/// policy decision (when to connect, retries, thermal plans) is made in
/// TypeScript (src/camera/session.ts), so the Android side implements the
/// same small surface.
public final class StephaniaCameraModule: Module {
  private let capture = CameraCapture()
  private var peer: SenderPeer?
  private var thermalObserver: NSObjectProtocol?
  private var pressure = "nominal"
  private var lastThermal = "nominal"
  private static var cachedDualProfiles: [String]?
  /// Mounted previews; re-attached whenever capture (and so the tracks) restarts.
  private let previews = NSHashTable<StephaniaCameraPreviewView>.weakObjects()

  private func refreshPreviews() {
    DispatchQueue.main.async {
      for v in self.previews.allObjects { v.attach(role: v.role, track: self.capture.track(v.role)?.track) }
    }
  }

  public func definition() -> ModuleDefinition {
    Name("StephaniaCamera")
    Events("onPeerState", "onThermalState", "onInterruption", "onCaptureError")

    OnCreate {
      capture.onInterruption = { [weak self] active, reason in
        self?.sendEvent("onInterruption", ["active": active, "reason": reason])
      }
      capture.onError = { [weak self] message in self?.sendEvent("onCaptureError", ["message": message]) }
      capture.onPressure = { [weak self] level in
        self?.pressure = level
        self?.emitThermal()
      }
      thermalObserver = NotificationCenter.default.addObserver(forName: ProcessInfo.thermalStateDidChangeNotification, object: nil, queue: nil) { [weak self] _ in
        self?.emitThermal()
      }
    }

    OnDestroy {
      if let o = thermalObserver { NotificationCenter.default.removeObserver(o) }
      peer?.close()
      Task { await self.capture.stop() }
    }

    AsyncFunction("capabilities") { (promise: Promise) in
      DispatchQueue.global(qos: .userInitiated).async {
        // Probing multi-cam formats configures devices; do it once per launch.
        let dual = Self.cachedDualProfiles ?? CameraCapture.dualProfiles()
        Self.cachedDualProfiles = dual
        promise.resolve(["roles": CameraCapture.roles(), "dualProfiles": dual, "hardwareH264": CameraCapture.hardwareH264()])
      }
    }

    Function("thermalState") { () -> String in self.currentThermal() }

    AsyncFunction("startCapture") { (mode: String, plans: [TrackPlanRecord], promise: Promise) in
      guard ["rear", "front", "dual"].contains(mode) else { promise.reject("E_CAMERA_MODE", "unknown camera mode \(mode)"); return }
      Task {
        do {
          self.peer?.close()
          self.peer = nil
          try await self.capture.start(mode: mode, plans: plans.map(\.plan))
          self.refreshPreviews()
          promise.resolve(nil)
        } catch let e as CameraCapture.Failure {
          if case .permission = e { promise.reject("E_CAMERA_PERMISSION", e.description) } else { promise.reject("E_CAMERA_CAPTURE", e.description) }
        } catch {
          promise.reject("E_CAMERA_CAPTURE", error.localizedDescription)
        }
      }
    }

    AsyncFunction("stopCapture") { (promise: Promise) in
      Task {
        self.peer?.close()
        self.peer = nil
        await self.capture.stop()
        self.refreshPreviews()
        promise.resolve(nil)
      }
    }

    AsyncFunction("createOffer") { (plans: [TrackPlanRecord], promise: Promise) in
      let tracks = self.capture.allTracks
      guard !tracks.isEmpty else { promise.reject("E_CAMERA_OFFER", "capture is not running"); return }
      Task {
        do {
          self.peer?.close()
          let peer = try SenderPeer(tracks: tracks, plans: plans.map(\.plan), captureHeights: self.capture.heights)
          peer.onState = { [weak self, weak peer] state in
            // Only the current peer reports; a replaced one is silent.
            guard let self, let peer, peer === self.peer else { return }
            self.sendEvent("onPeerState", ["state": state])
          }
          self.peer = peer
          let sdp = try await peer.makeOffer()
          promise.resolve(["sdp": sdp, "tracks": peer.trackMap])
        } catch {
          promise.reject("E_CAMERA_OFFER", String(describing: error))
        }
      }
    }

    AsyncFunction("setAnswer") { (sdp: String, promise: Promise) in
      guard let peer = self.peer else { promise.reject("E_CAMERA_ANSWER", "no offer in progress"); return }
      Task {
        do { try await peer.setAnswer(sdp); promise.resolve(nil) } catch { promise.reject("E_CAMERA_ANSWER", String(describing: error)) }
      }
    }

    AsyncFunction("applyPlans") { (plans: [TrackPlanRecord], promise: Promise) in
      do { try self.peer?.apply(plans.map(\.plan)); promise.resolve(nil) } catch { promise.reject("E_CAMERA_PLAN", String(describing: error)) }
    }

    AsyncFunction("closePeer") { (promise: Promise) in
      self.peer?.close()
      self.peer = nil
      promise.resolve(nil)
    }

    View(StephaniaCameraPreviewView.self) {
      Prop("camera") { (view: StephaniaCameraPreviewView, role: String) in
        self.previews.add(view)
        view.attach(role: role, track: self.capture.track(role)?.track)
      }
      Prop("mirrored") { (view: StephaniaCameraPreviewView, mirrored: Bool?) in
        view.mirrored = mirrored ?? false
      }
    }
  }

  /// The hotter of the system thermal state and the capture pressure (§12
  /// "ProcessInfo thermal/capture pressure").
  private func currentThermal() -> String {
    let order = ["nominal", "fair", "serious", "critical"]
    let system: String
    switch ProcessInfo.processInfo.thermalState {
    case .nominal: system = "nominal"
    case .fair: system = "fair"
    case .serious: system = "serious"
    default: system = "critical"
    }
    return (order.firstIndex(of: pressure) ?? 0) > (order.firstIndex(of: system) ?? 0) ? pressure : system
  }

  private func emitThermal() {
    let state = currentThermal()
    guard state != lastThermal else { return }
    lastThermal = state
    sendEvent("onThermalState", ["state": state])
  }
}
