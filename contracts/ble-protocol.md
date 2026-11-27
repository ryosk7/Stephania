# Stephania BLE provisioning protocol — version 3

Implementations:
- **Device core:** `src/provisioning.rs`, `src/provisioning_gatt.rs`, `src/bin/stephania-provision.rs`
- **BlueZ adapter:** `services/ble`
- **Phone:** `packages/provisioning-client`, `apps/mobile`
- **Independent test client:** `tests/ble/client.py`

Status: proof-of-concept contract. It still needs independent security review and physical testing. Do not call this production enrollment yet.

Version 3 replaced the v2 setup secret (a QR code made on a web page and copied onto the card) with a first-come claim. Version 2 had replaced the earlier multi-characteristic design (v1, never released) to remove common BlueZ and Linux BLE failure modes by design; see [BlueZ robustness](#bluez-robustness).

## Principles
- **Ownership is first come:** an unclaimed device belongs to the first phone that claims it. There is nothing to copy, print or scan. Whoever can write the microSD card controls the device anyway, and rewriting the card (or a factory reset with physical confirmation) makes it claimable again.
- **Key agreement, not a shared secret:** every `pairing.challenge` creates a fresh X25519 key on the device. The phone sends its own X25519 public key with the claim, and both sides derive the owner key from the shared secret. The owner key is never sent. A passive listener during the claim learns nothing; an active attacker in radio range at that moment could take the device first, and the owner then sees the device as taken in the app and rewrites the card.
- **No pairing or bonding:** the protocol never requires BLE pairing or bonding. Authentication (HMAC with the owner key) and confidentiality (ChaCha20-Poly1305 sealing of Wi-Fi credentials) are at the application layer. BLE link encryption is off by default (`STEPHANIA_BLE_REQUIRE_ENCRYPTION=1` enables it).
- **Ownership sticks:** a claimed device cannot be re-claimed over BLE; factory reset requires physical confirmation. A corrupt ownership file becomes `RECOVERY_REQUIRED`, never `UNCLAIMED`.
- **Two independent state machines:** ownership (`UNCLAIMED / PAIRING / CLAIMED / RECOVERY_REQUIRED`) and network (`DISCONNECTED / SCANNING / CONNECTING / CONNECTED / FAILED`). A Wi-Fi failure never changes ownership.
- **Wired first:** `network.status` reports `wired`. With an Ethernet cable connected the app skips Wi-Fi setup.

## GATT layout (fixed for v3)
Service `7c3b0200-5d1e-4a6f-9b0e-3a1f2c4d5e60`. Its UUID changes with every protocol major version, so a phone never applies a cached layout from another version.

| UUID | Name | Properties | Content |
|---|---|---|---|
| `7c3b0201-…` | Info | read | Static `{"v":3,"p":"steph"}` (19 bytes; `p` is a short product tag, Stephania). It fits a minimum-MTU read, so no long read is ever needed. |
| `7c3b0202-…` | Rpc | write (with response), indicate | Request fragments in; response fragments out. |

The advertisement carries the 128-bit service UUID and the name `Stephania-XXXX` (scan response if needed). It uses legacy advertising only.

## Framing
A fragment is `[message_id u8, index u8, count u8, chunk…]` and is at most the ATT payload (MTU − 3, minimum 20).
- **Limits:**
  - **Message size:** at most 4096 bytes, in at most 255 fragments. That is enough for 4 KiB even at the minimum MTU.
  - **Concurrency:** at most 4 messages in flight per peer.
  - **Reassembly:** times out after 10 s.
- **Receiver rules:**
  - Fragments may arrive out of order.
  - Identical duplicates are ignored.
  - Conflicting duplicates, inconsistent counts and oversize messages discard that message.
  - A device that receives a framing violation answers `{"error":"malformed"}` (or the matching code) instead of staying silent.

**Exchange:**
1. The client subscribes to indications on Rpc **before** writing.
2. The client writes every fragment of a request with write-with-response.
3. The device indicates the response, fragmented under the **request's message id**. Each indication is confirmed by the client's ATT stack before the next one is sent.

The client ignores indications for message ids it is not waiting for.

## Requests and responses
A request is a UTF-8 JSON object with `op`; unknown keys are rejected. A failed request answers `{"error":"<code>"}`. The `error` key is reserved for failures; status values use `last_error` / `scan_error`.

| op | Fields | Response | Replayed |
|---|---|---|---|
| `device.info` | — | `model, firmware, protocol:3, ownership, network` | no |
| `pairing.challenge` | — | `{state:"PAIRING", nonce, device_key}` while unclaimed (a fresh X25519 key each time; the latest challenge wins, 120 s), else `{state}` | yes |
| `pairing.claim` | `owner_id, phone_key` | `{state:"CLAIMED", confirm}` (persisted before the response; `bad_key` for a key with no usable shared secret, `not_claimable` once owned) | yes |
| `operation.challenge` | — | `{nonce}` (single use, 120 s) | yes |
| `wifi.scan` | `proof` (payload `""`) | `{networks:[{ssid,rssi,security}]}`: up to 40, strongest per SSID, `scan_error` on failure | yes |
| `wifi.configure` | `sealed, proof` (payload = `sealed`) | `{state:"CONNECTED"\|"FAILED"}` | yes |
| `network.status` | — | `{state, wired, last_error?}`; `wired` is true while Ethernet is connected | no |
| `management.endpoint` | — | `{available}`: whether LAN TLS is set up (unauthenticated hint; carries no URL or pin) | no |
| `management.enroll` | `proof` (payload `""`) | `{sealed}`; plaintext `{token, token_id, urls, ca_sha256}`; `management_unavailable` until the device PKI has published an endpoint | yes |

**Replay:** for operations marked "yes", the device caches the response for 60 s (8 entries), keyed by SHA-256 of `message_id ‖ request bytes`. An identical retry, even from a new BLE address after reconnecting, returns the original response without re-running the operation. Clients **must** retry with the same message id and identical bytes after a transport failure. A new id is a new request.

## Cryptography
`MAC(key, f1, f2, …) = HMAC-SHA256(key, u32be(len f1) ‖ f1 ‖ …)`

| Value | Definition |
|---|---|
| shared | `X25519(device private, phone_key) = X25519(phone private, device_key)`; all-zero results are refused (`bad_key`) |
| owner key | `MAC(shared, "stephania-owner-key-v2", nonce, owner_id, device_key, phone_key)`: derived on both sides, never sent |
| confirm | `MAC(owner_key, "stephania-claim-confirm-v1", nonce)`: the phone checks it to know both sides hold the same key |
| operation proof | `MAC(owner_key, "stephania-operation-v1", op_nonce, operation, payload)` |
| sealed | `ChaCha20-Poly1305(key = MAC(owner_key, "stephania-seal-v1"), nonce = op_nonce[0..12], aad = operation).encrypt(plaintext)` |

`management.enroll` seals its *response* the same way (operation `"management.enroll"`, the request carries no sealed payload, so the nonce is still used once). The phone keeps `token` in secure storage and accepts a LAN server only if its certificate chain ends in the CA whose DER SHA-256 is `ca_sha256`. The device keeps only SHA-256(token); a factory reset clears every token.

The Wi-Fi plaintext is `{"ssid", "security":"open"|"wpa-psk", "psk"?}`, with SSID 1–32 bytes and PSK 8–63 printable characters or 64 hex. The operation nonce is single-use, so every seal uses a unique nonce. `owner_id` is 1–64 characters of `[A-Za-z0-9._-]`. After 5 failed proofs or bad keys the device locks out for 300 s. If storing the new owner fails, the claim is rolled back and the device stays unclaimed.

Vectors are computed independently in Python (`fixtures/ble/generate.py`, `--check` to compare): `fixtures/ble/claim-vectors.json` and `fixtures/ble/seal-vectors.json`, verified by Rust and TypeScript.

## BlueZ robustness
| Common failure | Design response |
|---|---|
| Pairing/bonding failures: AuthenticationTimeout, stale bonds after phone reset, iOS "Peer removed pairing information", Just Works repair bugs | No pairing or bonding is ever required. Credentials are sealed and requests authenticated at the application layer. `JustWorksRepairing=always` in case a phone pairs anyway. |
| Long-read (Read Blob) quirks, offset handling, read side effects | No dynamic reads. Info is static and fits one minimum-MTU read; everything else goes through write + indicate. |
| Lost, coalesced or duplicated notifications | Indications, confirmed per fragment. The message id plus count, and identical-duplicate tolerance, make reassembly robust. |
| Disconnect after a mutation but before the response | Replay cache. The client reconnects and resends identically, and the operation runs once. |
| Phones caching an old GATT database (Service Changed not honoured) | Minimal fixed layout. A new service UUID per major version. Static identity address (`Privacy=off`). |
| MTU misreporting or mismatch (bleak/BlueZ report 23, negotiated 517) | Sender fragments to the MTU it observes. Receivers accept any fragment size up to their own ATT payload. Everything works at 23. |
| Several centrals connecting or interleaving writes | Per-peer reassembly. `stephania-ble` serves one active central (others get `InProgress` until it disconnects or idles 30 s). Partial transfers are dropped on disconnect. |
| bluetoothd restart, adapter reset or removal, advertising silently dropped, or "registered but not advertising" after a power transition | `stephania-ble` exits on daemon or adapter loss, and systemd (`Restart=always`, `BindsTo=bluetooth.service`) re-registers it. It powers the adapter back on if it is off. It re-registers the advertisement when the controller reports Powered, when BlueZ drops it, and every 30 s while no central is connected. Verified in CI by restarting bluetoothd mid-test (found by that test). |
| On-board CYW43455 limits: BT 5.0 mandatory features only; reported drops and packet loss | No optional BT5 feature is used: legacy advertising, 1M PHY, no DLE requirement. `STEPHANIA_BLE_ADAPTER=auto` prefers an external USB controller (e.g. an nRF52840 dongle running a Zephyr HCI controller) with no code change. |
| BlueZ version differences and experimental APIs | Only stable `GattManager1` / `LEAdvertisingManager1`. The OS image pins BlueZ. CI runs Ubuntu 24.04's BlueZ (5.72). |
| Running BLE code as root | `stephania-ble` runs as an unprivileged user with a scoped D-Bus policy. It holds no secrets; the device core is a separate process. |

If BlueZ itself proves unsuitable on hardware, the adapter process can be replaced without touching the device core or the app. The options are a different host stack over HCI, or a BLE MCU over UART. Only `services/ble` changes.

## Verification (CI)
- **Core rules:** Rust unit tests cover replay, interleaved peers, sealing, worst-case sizes and persistence ordering.
- **Device core over its socket:** `tests/provision_socket.rs` runs the real binary, including reconnect replay.
- **TypeScript client and app transport:** both are tested against the real binary. The tests lose and duplicate responses on purpose and require recovery.
- **`ble-e2e`, real BlueZ:** runs on two virtual LE controllers (`btvirt`).
  - **Flow:** the Python client has a bad key refused, claims, replays, scans, fails and then succeeds a join, and has an unauthenticated operation rejected.
  - **Self-heal:** bluetoothd is then restarted, and the supervised adapter must self-heal and serve the same owned device.
  - **Variants:** runs at the negotiated MTU and with 20-byte fragments.

Not verified: real radios, phones, range and interference, and the on-board CYW43455.

## Open items
- **Security:** independent security review; owner credential rotation and revocation; multiple owners.
- **Networking:** NetworkManager D-Bus adapter with PSK persistence; captive-portal detection.
- **Phone and hardware:** phone-side behaviour on real iOS/Android; physical range tests.
