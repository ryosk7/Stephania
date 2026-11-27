Pod::Spec.new do |s|
  s.name           = 'StephaniaAccessorySetup'
  s.version        = '0.1.0'
  s.summary        = 'One-tap Stephania setup with AccessorySetupKit'
  s.description    = 'Shows the iOS 18 AccessorySetupKit picker for the Stephania provisioning service.'
  s.author         = 'Ryosuke Uchida'
  s.homepage       = 'https://docs.expo.dev/modules/'
  s.platforms      = {
    :ios => '16.4'
  }
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'
  # iOS 18 only; older systems use the app-drawn sheet.
  s.weak_frameworks = 'AccessorySetupKit'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
  }

  s.source_files = "*.swift"
end
