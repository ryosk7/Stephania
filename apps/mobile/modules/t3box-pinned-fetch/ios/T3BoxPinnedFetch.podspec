Pod::Spec.new do |s|
  s.name           = 'T3BoxPinnedFetch'
  s.version        = '0.1.0'
  s.summary        = 'Pinned HTTPS client for the T3Box device CA'
  s.description    = 'URLSession requests that trust only the device CA enrolled over BLE.'
  s.author         = 'Ryosuke Uchida'
  s.homepage       = 'https://docs.expo.dev/modules/'
  s.platforms      = {
    :ios => '16.4'
  }
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  # Swift/Objective-C compatibility
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
  }

  # The SwiftPM package next to this file holds the trust logic and its
  # tests (`swift test`); only its sources go into the pod.
  s.source_files = ["*.swift", "PinnedTrust/Sources/**/*.swift"]
end
