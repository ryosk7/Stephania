# T3Box Connect (Expo)

Phone app for BLE ownership enrollment and Wi-Fi setup (Phase 5). iOS first; Android shares the code.

Flow: scan the setup QR from the setup page (`tools/setup-page`) → find nearby T3Box over BLE (service `7c3b0001-…`) → claim with the setup secret → owner key stored in Keychain/Keystore (`WHEN_UNLOCKED_THIS_DEVICE_ONLY`) → scan Wi-Fi → join.

- Protocol logic: `packages/provisioning-client` (shared, tested against the Rust device core).
- `src/ble/transport.ts`: react-native-ble-plx transport. It fragments writes to the negotiated ATT payload, subscribes before writing, and reassembles notified responses under the request's message id. Timeouts are 15 s by default, 20 s for scan and 45 s for join.
- `src/app/`: Expo Router screens (`index`, `scan`, `devices`, `wifi`, `dashboard`, `open-on-pc`).
- `modules/t3box-pinned-fetch`: local Expo module (Swift/Kotlin) for HTTPS that trusts only the enrolled device CA (docs/AUTH.md). Trust logic: `ios/PinnedTrust` (SwiftPM, `swift test`) and `android/.../PinnedHttp.kt` (JUnit against a TLS server using real `t3box-pki` certificates).
- `src/net/device.ts`: management API client over the pinned module (URL fallback, error envelope). The dashboard shows status, starts/stops the stream, switches scenes (cut or 0.5 s fade) and mutes audio inputs.

## Run
Needs a **development build**, because Expo Go lacks the BLE native module:
```bash
npm ci                       # also run `npm ci` in packages/provisioning-client
npx expo run:ios --device    # or: npx eas-cli@latest build --profile development
```

## Verified here (CI)
- `npm run typecheck`
- Android: `expo prebuild` + Gradle debug APK with the native module; `PinnedHttpTest` (JUnit) against a local TLS server using certificates from the real device PKI: right pin accepted, other pin, uncertified name and plain HTTP refused
- iOS: `swift test` of `PinnedTrust` (Security.framework) with device-PKI certificate fixtures: certified name/address accepted; other names, pins, foreign leaves and expired certificates refused
- Vitest: the BLE transport against the real Rust `t3box-provision` binary, through a fake ble-plx that follows the adapter contract (notified, fragmented responses at the minimum MTU)
- `expo export` for iOS and Android (Metro bundling resolves all imports)

## Not verified
Real radios and devices are untested: iOS/Android BLE behaviour, MTU negotiation, background/foreground interruptions, the camera QR scan and permission prompts. The BlueZ GATT adapter on the Pi doesn't exist yet, so there is nothing real to connect to. No Maestro tests yet. The full iOS app build runs only on demand (workflow "Mobile iOS build"); the Android debug APK and both platforms' trust logic are built and tested on PRs that touch native code.
