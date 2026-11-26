//! t3box-ble: BlueZ GATT adapter for T3Box provisioning (protocol v2).
//!
//! A thin process with no secrets and no protocol decisions: it publishes
//! one service with two characteristics and forwards request fragments to
//! `t3box-provision` (`gatt.request`, docs/IPC.md framing).
//!
//! BlueZ robustness measures implemented here (contracts/ble-protocol.md
//! § "BlueZ robustness"):
//! - Minimal, fixed GATT layout: a static 19-byte Info read (fits a
//!   minimum-MTU read, never a long read) and one RPC characteristic
//!   (write-with-response + indicate). No dynamic reads at all.
//! - Responses are sent as **indications**: each fragment waits for the
//!   client's ATT confirmation before the next is sent.
//! - No BLE pairing or bonding is required (link encryption off by default;
//!   confidentiality and authentication are at the application layer).
//! - One active central at a time; other centrals get `InProgress` until the
//!   active one disconnects or idles out. Partial transfers are dropped on
//!   disconnect.
//! - Self-healing: if bluetoothd restarts, the adapter disappears or is
//!   powered off, the process exits non-zero so systemd restarts it and
//!   re-registers the application and advertisement. Advertising is checked
//!   periodically and re-registered if BlueZ dropped it.
//! - Adapter selection prefers a USB controller (external dongle) over the
//!   on-board UART controller when `T3BOX_BLE_ADAPTER=auto`.

use bluer::adv::{Advertisement, AdvertisementHandle, Type as AdvType};
use bluer::gatt::local::{
    Application, Characteristic, CharacteristicNotifier, CharacteristicNotify, CharacteristicNotifyMethod,
    CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, ReqError, Service,
};
use bluer::{Adapter, Address, DeviceEvent, DeviceProperty, SessionEvent};
use futures::{FutureExt, StreamExt};
use serde_json::{json, Value};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use t3box::ipc::{read_frame, write_frame, IO_TIMEOUT};
use t3box::provisioning::{fragment, hex, MIN_ATT_PAYLOAD};
use t3box::provisioning_gatt::{INFO_UUID, INFO_VALUE, RPC_UUID, SERVICE_UUID};

/// An idle central stops blocking others after this long.
const PEER_IDLE: Duration = Duration::from_secs(30);
const WATCHDOG_PERIOD: Duration = Duration::from_secs(10);

/// Socket client to t3box-provision; one long-lived connection, reconnected
/// once on failure (provision restarts are expected under systemd).
struct Provision {
    path: PathBuf,
    stream: Option<UnixStream>,
    next: u64,
}

impl Provision {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        for attempt in 0..2 {
            if self.stream.is_none() {
                let s = UnixStream::connect(&self.path).map_err(|e| format!("provision_unavailable: {}", e.kind()))?;
                // A Wi-Fi join may take ~30 s inside the device core.
                let _ = s.set_read_timeout(Some(IO_TIMEOUT * 10));
                let _ = s.set_write_timeout(Some(IO_TIMEOUT));
                self.stream = Some(s);
            }
            self.next += 1;
            let body = serde_json::to_vec(&json!({"version":1,"request_id":format!("ble-{}", self.next),"method":method,"params":params}))
                .map_err(|_| "internal_error".to_string())?;
            let stream = self.stream.as_mut().expect("connected");
            let result = write_frame(stream, &body).map_err(|e| e.to_string())
                .and_then(|_| read_frame(stream).map_err(|e| e.message).and_then(|f| f.ok_or_else(|| "closed".to_string())));
            match result {
                Ok(frame) => {
                    let v: Value = serde_json::from_slice(&frame).map_err(|_| "invalid_response".to_string())?;
                    return if v["ok"] == true { Ok(v["result"].clone()) } else {
                        Err(v["error"]["code"].as_str().unwrap_or("internal_error").to_string())
                    };
                }
                Err(_) if attempt == 0 => self.stream = None,
                Err(e) => { self.stream = None; return Err(format!("provision_unavailable: {e}")); }
            }
        }
        unreachable!()
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) { return None; }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// ATT payload for indications: MTU − 3, within protocol bounds.
pub fn att_payload(mtu: u16) -> usize {
    (mtu as usize).saturating_sub(3).clamp(MIN_ATT_PAYLOAD, 509)
}

/// Which central may currently talk to the device.
#[derive(Debug, Default)]
pub struct PeerGate {
    active: Option<(Address, Instant)>,
}

impl PeerGate {
    /// Admit `peer` if no other central is active (or the active one idled).
    /// Returns the displaced peer when another central idled out.
    pub fn admit(&mut self, peer: Address, now: Instant) -> Result<Option<Address>, ReqError> {
        match self.active {
            Some((current, _)) if current == peer => { self.active = Some((peer, now)); Ok(None) }
            Some((_, last)) if now.duration_since(last) < PEER_IDLE => Err(ReqError::InProgress),
            previous => { self.active = Some((peer, now)); Ok(previous.map(|(p, _)| p)) }
        }
    }

    pub fn release(&mut self, peer: Address) -> bool {
        if matches!(self.active, Some((p, _)) if p == peer) { self.active = None; true } else { false }
    }

    pub fn active(&self) -> Option<Address> { self.active.map(|(p, _)| p) }
}

struct Shared {
    provision: Mutex<Provision>,
    gate: Mutex<PeerGate>,
    watched: Mutex<std::collections::HashSet<Address>>,
    notifier: tokio::sync::Mutex<Option<CharacteristicNotifier>>,
    adapter: Adapter,
}

async fn provision_call(shared: &Arc<Shared>, method: &'static str, params: Value) -> Result<Value, String> {
    let shared = Arc::clone(shared);
    tokio::task::spawn_blocking(move || shared.provision.lock().unwrap_or_else(|e| e.into_inner()).call(method, params))
        .await
        .map_err(|_| "internal_error".to_string())?
}

/// Watch a central's connection (once per address) and release it on
/// disconnect, dropping its partial transfer in the device core.
fn watch_disconnect(shared: &Arc<Shared>, peer: Address) {
    if !shared.watched.lock().unwrap_or_else(|e| e.into_inner()).insert(peer) {
        return;
    }
    let shared = Arc::clone(shared);
    tokio::spawn(async move {
        if let Ok(device) = shared.adapter.device(peer) {
            if let Ok(events) = device.events().await {
                let mut events = Box::pin(events);
                while let Some(ev) = events.next().await {
                    if let DeviceEvent::PropertyChanged(DeviceProperty::Connected(false)) = ev { break; }
                }
            }
        }
        shared.watched.lock().unwrap_or_else(|e| e.into_inner()).remove(&peer);
        shared.gate.lock().unwrap_or_else(|e| e.into_inner()).release(peer);
        let _ = provision_call(&shared, "gatt.disconnect", json!({"peer": peer.to_string()})).await;
    });
}

async fn on_write(shared: Arc<Shared>, value: Vec<u8>, peer: Address, mtu: u16, offset: u16) -> Result<(), ReqError> {
    if value.is_empty() || offset != 0 {
        return Err(ReqError::InvalidValueLength);
    }
    let displaced = shared.gate.lock().unwrap_or_else(|e| e.into_inner()).admit(peer, Instant::now())?;
    if let Some(previous) = displaced {
        let _ = provision_call(&shared, "gatt.disconnect", json!({"peer": previous.to_string()})).await;
    }
    watch_disconnect(&shared, peer);
    let message_id = value[0];
    let r = provision_call(&shared, "gatt.request", json!({"peer": peer.to_string(), "fragment_hex": hex(&value)})).await;
    let response = match r {
        Ok(v) => match v["response_hex"].as_str() {
            Some(h) => unhex(h).ok_or(ReqError::Failed)?,
            None => return Ok(()), // more fragments expected
        },
        Err(_) => return Err(ReqError::Failed),
    };
    let att = att_payload(mtu);
    // Indicate after the write is acknowledged; each fragment is confirmed.
    tokio::spawn(async move {
        let Ok(frags) = fragment(message_id, &response, att) else { return };
        let mut guard = shared.notifier.lock().await;
        let Some(n) = guard.as_mut() else {
            eprintln!("t3box-ble: response dropped, client not subscribed to indications");
            return;
        };
        for f in frags {
            if let Err(e) = n.notify(f).await {
                eprintln!("t3box-ble: indication failed: {e}");
                *guard = None;
                return;
            }
        }
    });
    Ok(())
}

fn rpc_characteristic(shared: &Arc<Shared>, encrypt: bool) -> Characteristic {
    let write_shared = Arc::clone(shared);
    let notify_shared = Arc::clone(shared);
    Characteristic {
        uuid: RPC_UUID.parse().expect("uuid"),
        write: Some(CharacteristicWrite {
            write: true,
            encrypt_write: encrypt,
            method: CharacteristicWriteMethod::Fun(Box::new(move |value, req| {
                on_write(Arc::clone(&write_shared), value, req.device_address, req.mtu, req.offset).boxed()
            })),
            ..Default::default()
        }),
        notify: Some(CharacteristicNotify {
            indicate: true,
            method: CharacteristicNotifyMethod::Fun(Box::new(move |notifier| {
                let shared = Arc::clone(&notify_shared);
                async move { *shared.notifier.lock().await = Some(notifier); }.boxed()
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn info_characteristic(encrypt: bool) -> Characteristic {
    Characteristic {
        uuid: INFO_UUID.parse().expect("uuid"),
        read: Some(CharacteristicRead {
            read: true,
            encrypt_read: encrypt,
            fun: Box::new(|req| async move {
                // Static and ≤ 22 bytes: never needs a long read.
                Ok(INFO_VALUE.get(req.offset as usize..).ok_or(ReqError::InvalidOffset)?.to_vec())
            }.boxed()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// `auto`: prefer a USB-attached controller (external dongle) over the
/// on-board one; otherwise BlueZ's default adapter.
async fn pick_adapter(session: &bluer::Session) -> bluer::Result<Adapter> {
    match std::env::var("T3BOX_BLE_ADAPTER").unwrap_or_else(|_| "auto".into()).as_str() {
        "auto" | "" => {
            for name in session.adapter_names().await? {
                let real = std::fs::canonicalize(format!("/sys/class/bluetooth/{name}/device")).unwrap_or_default();
                if real.to_string_lossy().contains("/usb") {
                    return session.adapter(&name);
                }
            }
            session.default_adapter().await
        }
        name => session.adapter(name),
    }
}

async fn advertise(adapter: &Adapter, name: &str) -> bluer::Result<AdvertisementHandle> {
    adapter.advertise(Advertisement {
        advertisement_type: AdvType::Peripheral,
        // 128-bit UUID (18 bytes) + flags (3) fit a legacy 31-byte
        // advertisement; BlueZ moves the name to the scan response if needed.
        service_uuids: [SERVICE_UUID.parse().expect("uuid")].into_iter().collect(),
        local_name: Some(name.to_owned()),
        discoverable: Some(true),
        ..Default::default()
    }).await
}

fn fatal(kind: bluer::ErrorKind, message: &str) -> bluer::Error {
    bluer::Error { kind, message: message.into() }
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        // Non-zero exit: systemd restarts us and we re-register cleanly.
        eprintln!("t3box-ble: fatal: {e}");
        std::process::exit(2);
    }
}

async fn run() -> bluer::Result<()> {
    let socket = std::env::var_os("T3BOX_PROVISION_SOCKET").map(PathBuf::from)
        .unwrap_or_else(|| "/run/t3box-provision/provisioning.sock".into());
    // Off by default: requiring link encryption forces BLE pairing/bonding,
    // the most common source of interoperability failures. The protocol
    // authenticates and seals at the application layer instead.
    let encrypt = std::env::var("T3BOX_BLE_REQUIRE_ENCRYPTION").as_deref() == Ok("1");
    let session = bluer::Session::new().await?;
    let adapter = pick_adapter(&session).await?;
    adapter.set_powered(true).await?;
    let address = adapter.address().await?;
    let name = format!("T3Box-{:02X}{:02X}", address.0[4], address.0[5]);

    let shared = Arc::new(Shared {
        provision: Mutex::new(Provision { path: socket, stream: None, next: 0 }),
        gate: Mutex::new(PeerGate::default()),
        watched: Mutex::new(Default::default()),
        notifier: tokio::sync::Mutex::new(None),
        adapter: adapter.clone(),
    });
    let app = Application {
        services: vec![Service {
            uuid: SERVICE_UUID.parse().expect("uuid"),
            primary: true,
            characteristics: vec![info_characteristic(encrypt), rpc_characteristic(&shared, encrypt)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let _app = adapter.serve_gatt_application(app).await?;
    let mut adv = advertise(&adapter, &name).await?;
    eprintln!("t3box-ble: advertising {name} on {} (link encryption {})", adapter.name(), if encrypt { "required" } else { "off" });

    let adapter_name = adapter.name().to_owned();
    let mut session_events = Box::pin(session.events().await?);
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut tick = tokio::time::interval(WATCHDOG_PERIOD);
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = term.recv() => return Ok(()),
            ev = session_events.next() => match ev {
                Some(SessionEvent::AdapterRemoved(n)) if n == adapter_name => return Err(fatal(bluer::ErrorKind::NotFound, "adapter removed")),
                None => return Err(fatal(bluer::ErrorKind::Failed, "bluetoothd connection lost")),
                _ => {}
            },
            _ = tick.tick() => {
                // Errors here mean bluetoothd restarted or the adapter vanished.
                if !adapter.is_powered().await? {
                    return Err(fatal(bluer::ErrorKind::NotReady, "adapter powered off"));
                }
                let idle = shared.gate.lock().unwrap_or_else(|e| e.into_inner()).active().is_none();
                if idle && adapter.active_advertising_instances().await? == 0 {
                    eprintln!("t3box-ble: advertising was dropped by BlueZ; re-registering");
                    drop(adv);
                    adv = advertise(&adapter, &name).await?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn att_payload_bounds() {
        assert_eq!(att_payload(23), 20);
        assert_eq!(att_payload(0), 20);
        assert_eq!(att_payload(247), 244);
        assert_eq!(att_payload(517), 509);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(unhex("00ff"), Some(vec![0, 255]));
        assert_eq!(unhex("0"), None);
        assert_eq!(unhex("zz"), None);
    }

    #[test]
    fn single_active_central_with_idle_takeover() {
        let a = Address([1, 2, 3, 4, 5, 6]);
        let b = Address([6, 5, 4, 3, 2, 1]);
        let t = Instant::now();
        let mut g = PeerGate::default();
        assert_eq!(g.admit(a, t), Ok(None));
        assert_eq!(g.admit(a, t + Duration::from_secs(5)), Ok(None));
        assert_eq!(g.admit(b, t + Duration::from_secs(10)), Err(ReqError::InProgress));
        // a idles out: b takes over and a's partial state must be dropped.
        assert_eq!(g.admit(b, t + Duration::from_secs(5) + PEER_IDLE), Ok(Some(a)));
        assert!(!g.release(a));
        assert!(g.release(b));
        assert_eq!(g.admit(a, t + Duration::from_secs(100)), Ok(None));
    }

    #[test]
    fn info_value_never_needs_a_long_read() {
        assert!(INFO_VALUE.len() <= 22);
    }
}
