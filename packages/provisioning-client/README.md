# @t3box/provisioning-client

Phone-side implementation of `contracts/ble-protocol.md` for the Expo app (Phase 5). Transport-agnostic: the app supplies a `BleTransport` backed by react-native-ble-plx (not written yet); tests use a Unix-socket transport against the real Rust `t3box-provision` binary.

- `ProvisioningSession.claim(setupSecret, ownerId)` → owner key (store in Keychain/Keystore; never transmitted)
- `scan()`, `configureWifi()`, `networkStatus()` — owner-authenticated
- Pure-JS HMAC-SHA256 (`@noble/hashes`), verified against `contracts/fixtures/ble/hmac-vectors.json`

```bash
cargo build --no-default-features --features ci-virtual --bin t3box-provision
T3BOX_PROVISION_BIN=../../target/debug/t3box-provision npm test
```
Not yet: the Expo app, ble-plx transport, QR/label secret entry UI (pending R02 decision), Maestro tests.
