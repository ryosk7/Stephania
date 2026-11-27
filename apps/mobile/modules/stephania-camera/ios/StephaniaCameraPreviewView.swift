import ExpoModulesCore
import UIKit
import WebRTC

/// Native preview of one camera's local track (Metal; frames never reach JS).
/// Shows exactly what is fed to the encoder, before any network.
final class StephaniaCameraPreviewView: ExpoView {
  private let video = RTCMTLVideoView()
  // Strong: the ObjC wrapper owns the renderer adapter (released → no frames).
  private var track: RTCVideoTrack?
  private(set) var role = ""

  var mirrored = false {
    didSet { video.transform = mirrored ? CGAffineTransform(scaleX: -1, y: 1) : .identity }
  }

  required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
    clipsToBounds = true
    backgroundColor = .black
    video.videoContentMode = .scaleAspectFill
    addSubview(video)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    video.frame = bounds
  }

  func attach(role: String, track: RTCVideoTrack?) {
    self.role = role
    if self.track === track { return }
    self.track?.remove(video)
    self.track = track
    track?.add(video)
  }

  deinit {
    track?.remove(video)
  }
}
