# T3Box Connect (Expo)

Phone app for BLE ownership enrollment and Wi-Fi setup (Phase 5). iOS first; Android shares the code.

Flow: scan the setup QR from the setup page (`tools/setup-page`) → find nearby T3Box over BLE (service `7c3b0001-…`) → claim with the setup secret → owner key stored in Keychain/Keystore (`WHEN_UNLOCKED_THIS_DEVICE_ONLY`) → scan Wi-Fi → join.

- Protocol logic: `packages/provisioning-client` (shared, tested against the Rust device core).
- `src/ble/transport.ts`: react-native-ble-plx transport. It fragments writes to the negotiated ATT payload, subscribes before writing, and reassembles notified responses under the request's message id. Timeouts are 15 s by default, 20 s for scan and 45 s for join.
- `src/app/`: Expo Router screens (`index`, `scan`, `devices`, `wifi`).

## Run
Needs a **development build**, because Expo Go lacks the BLE native module:
```bash
npm ci                       # also run `npm ci` in packages/provisioning-client
npx expo run:ios --device    # or: npx eas-cli@latest build --profile development
```

## Verified here (CI)
- `npm run typecheck`
- Vitest: the BLE transport against the real Rust `t3box-provision` binary, through a fake ble-plx that follows the adapter contract (notified, fragmented responses at the minimum MTU)
- `expo export` for iOS and Android (Metro bundling resolves all imports)

## Not verified
Real radios and devices are untested: iOS/Android BLE behaviour, MTU negotiation, background/foreground interruptions, the camera QR scan and permission prompts. The BlueZ GATT adapter on the Pi doesn't exist yet, so there is nothing real to connect to. No Maestro tests or management-endpoint handoff yet (that needs trusted TLS).
