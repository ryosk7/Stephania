# Engine control IPC (version 1)

Contract: `contracts/ipc.schema.json`. Server: `src/ipc.rs`. Client: `services/control/lib/engine_client.rb`.
Shared conformance fixtures: `contracts/fixtures/ipc/` and `contracts/fixtures/scenes/` (run by both Rust and Ruby suites).

## Transport
- Unix domain socket, default `/run/t3box/engine.sock`, override with `T3BOX_ENGINE_SOCKET` (`off` disables).
- Socket mode `0660`; access is restricted by owner/group and the systemd `RuntimeDirectory` (mode `0750`).
- Linux peer credentials (SO_PEERCRED) are checked on accept: root, the engine's own uid and uids listed in `T3BOX_IPC_ALLOWED_UIDS` (comma separated) are allowed; others receive `forbidden` and are closed. If credentials cannot be read on Linux the connection is refused. An invalid `T3BOX_IPC_ALLOWED_UIDS` disables the socket (engine keeps running).
- A pre-existing socket at the path is treated as stale and replaced. Any other file type makes binding fail; nothing else is deleted.
- Bind failure is logged and the engine keeps running (media plane never depends on management).
- The loopback HTTP API on `127.0.0.1:8787` remains as a transitional interface and must never be exposed to the LAN.

## Framing
`u32` big-endian length, then that many bytes of UTF-8 JSON. Length 0 or > 65536 is a framing error: the server answers once with `request_id: null` and closes the connection. Several requests may be sent sequentially on one connection; each receives exactly one response with the same `request_id`.

## Bounds
- 8 concurrent connections; the 9th receives `busy` and is closed.
- 5 s read/write timeout per connection; each connection runs on its own thread, never on a media thread.

## Messages
```json
{"version":1,"request_id":"req-001","method":"engine.status","params":{}}
{"version":1,"request_id":"req-001","ok":true,"result":{...}}
{"version":1,"request_id":"req-001","ok":false,"error":{"code":"invalid_params","message":"..."}}
```
Unknown top-level properties are rejected.

| Method | Params | Result / behaviour |
|---|---|---|
| `engine.status` | none | `instance_id` (random per process; change ⇒ engine restarted), `uptime_ms`, `state`, `streaming_requested`, `native_worker_running`, `youtube_ingest_confirmed` (always `false`), `restart_attempts`, `last_error` |
| `engine.capabilities` | none | `ipc_version`, `methods`, `scene_versions`, `program_apply:false`, `compositor:false`, `phone_camera:false`, `max_frame_bytes`, `instance_id` |
| `stream.start` | none | `accepted:true` — request taken; not ingest confirmation. `engine_start_failed` on configuration error |
| `stream.stop` | none | `accepted:true, streaming_requested:false` (cooperative stop) |
| `scene.validate` | `{scene}` | Independent engine validation of draft scene v1 |
| `scene.apply` | `{scene}` | Validates, then **fails closed** with `not_implemented`; Program unchanged until the compositor acknowledges frame-boundary application |

## Not yet implemented
Cancellation, idempotency keys, `source.*`, `audio.configure`, `metrics.snapshot`, server-pushed events.

## Restart reconciliation
The engine keeps no state across restarts; it always starts idle. The control service records the last seen `instance_id` and reports `engine_restarted: true` on `/api/v1/status` when it changes, so clients re-read state instead of assuming a stream is still running. Auto-resume after an engine crash is deliberately not implemented (§27: manual initial start; opt-in recovery policy later).
