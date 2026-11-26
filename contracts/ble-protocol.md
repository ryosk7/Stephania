# T3Box BLE provisioning protocol — version 1 (draft)

Status: **proof-of-concept contract.** Device core in `src/provisioning.rs`, characteristic semantics in `src/provisioning_gatt.rs`; BlueZ GATT adapter, NetworkManager adapter and the Expo client are not implemented. Initial secret delivery without monitor/keyboard is an open security PoC (risk R02). Do not describe this as production enrollment.

## Principles
- BLE proximity is not authentication. Ownership is proven with a per-device 128-bit setup secret (generated on first boot, file mode 0600, never transmitted) via HMAC-SHA256 challenge-response.
- No shared default credential. A claimed device cannot be re-claimed over BLE; factory reset requires physical confirmation.
- Ownership (`UNCLAIMED / PAIRING / CLAIMED / RECOVERY_REQUIRED`) is independent of network state (`DISCONNECTED / SCANNING / CONNECTING / CONNECTED / FAILED`). Wi-Fi failure never changes ownership. A corrupt ownership file becomes `RECOVERY_REQUIRED`, never `UNCLAIMED`.
- BLE link: LE Secure Connections required by the GATT adapter; application-layer proofs do not rely on it for authentication.

## Service and characteristics
Base UUID `7c3bXXXX-5d1e-4a6f-9b0e-3a1f2c4d5e60`; service `7c3b0001-…`.

| UUID suffix | Name | Props | Payload (UTF-8 JSON unless noted) |
|---|---|---|---|
| 0002 | ProtocolVersion | read | `{"version":1}` |
| 0003 | DeviceInformation | read | `{"model","firmware","ownership","network"}` (no secrets) |
| 0004 | PairingChallenge | read, notify | `{"state","nonce"?}` — nonce hex(32 B) while PAIRING |
| 0005 | PairingResponse | write | `{"owner_id","proof"}` |
| 0006 | WiFiScanRequest | write | `{}` (owner-authenticated, see below) |
| 0007 | WiFiScanResults | read, notify | `{"page","pages","networks":[{"ssid","rssi","security"}]}` ≤ 20 per page, ≤ 60 total, strongest per SSID; each read returns the next page (wraps); owner only |
| 0008 | WiFiConfiguration | write | authenticated envelope around `{"ssid","security":"open"|"wpa-psk","psk"?}` |
| 0009 | NetworkStatus | read, notify | `{"state","error"?}` |
| 000a | ManagementEndpoint | read | `{"urls":[…],"certificate_sha256"}` once trusted TLS exists; empty until then |
| 000b | DevicePublicKey | read | reserved for the TLS/identity PoC |
| 000c | OperationChallenge | read | `{"nonce"}` single-use, 120 s |

## Framing
Writes/notifications carry fragments `[message_id u8, index u8, count u8, chunk…]`, each ≤ ATT payload (MTU − 3, min 20). Message ≤ 4096 bytes, ≤ 64 fragments, ≤ 4 messages in flight, 10 s reassembly timeout. Out-of-order fragments are accepted; duplicates, inconsistent counts and oversize messages discard that message. A 4 KiB message needs an ATT payload ≥ 67 bytes; at the minimum MTU messages are limited to 64 × 17 bytes.

## Cryptography
`MAC(key, f1, f2, …) = HMAC-SHA256(key, u32be(len f1) ‖ f1 ‖ u32be(len f2) ‖ f2 ‖ …)`

| Value | Definition |
|---|---|
| claim proof | `MAC(setup_secret, "t3box-claim-v1", nonce, owner_id)` |
| owner key | `MAC(setup_secret, "t3box-owner-key-v1", nonce, owner_id)` — derived on both sides, never sent |
| operation proof | `MAC(owner_key, "t3box-operation-v1", op_nonce, operation, payload)` |

`owner_id`: 1–64 of `[A-Za-z0-9._-]`. Interop vectors: `contracts/fixtures/ble/hmac-vectors.json` (computed independently with Python `hmac`; verified by the Rust suite). The Expo client must pass them too.

## Flows
1. **Claim:** read PairingChallenge (device enters PAIRING, 120 s) → write PairingResponse → device persists CLAIMED durably, then notifies `CLAIMED`. 5 failed proofs → 300 s lockout.
2. **Authenticated operation:** read OperationChallenge → write `{"operation","payload_hex","proof"}` to the operation's characteristic. Nonce is consumed on every attempt; proof binds operation and payload.
3. **Wi-Fi:** authenticated `wifi.configure` → NetworkStatus CONNECTING → CONNECTED/FAILED. Credentials are validated (SSID 1–32 bytes, PSK 8–63 printable or 64 hex) before NetworkManager is touched and never logged.
4. **Reset:** only with physical confirmation (button held at boot); returns to UNCLAIMED and clears pairing state.

## Errors
`invalid_json, not_object, invalid_ssid, invalid_psk, invalid_security, not_claimable, no_pairing_in_progress, pairing_expired, locked_out, invalid_owner_id, bad_proof, not_owner, physical_confirmation_required` and framing errors `too_large, payload_too_small, malformed, inconsistent, duplicate`.

## Open items
Secret delivery (label/QR), PAKE evaluation vs. HMAC over a high-entropy secret, owner credential rotation/revocation, multiple owners, BlueZ adapter with LE Secure Connections enforcement, NetworkManager D-Bus adapter, captive-portal detection, Expo client and Maestro tests, physical range/interference testing.
