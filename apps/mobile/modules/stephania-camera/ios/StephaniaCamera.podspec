Pod::Spec.new do |s|
  s.name           = 'StephaniaCamera'
  s.version        = '0.1.0'
  s.summary        = 'Stephania phone camera sender'
  s.description    = 'AVCaptureMultiCamSession capture, VideoToolbox H.264 and WebRTC transport to a Stephania device.'
  s.author         = 'Ryosuke Uchida'
  s.homepage       = 'https://docs.expo.dev/modules/'
  s.platforms      = {
    :ios => '16.4'
  }
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'
  # Prebuilt libwebrtc (BSD); pinned to the version the SwiftPM core tests against.
  s.dependency 'WebRTC-SDK', '150.7871.09'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
  }

  # The SwiftPM package next to this file holds the platform-neutral sender
  # core and its tests (`swift test`); only its sources go into the pod.
  s.source_files = ["*.swift", "CameraSender/Sources/**/*.swift"]
end
