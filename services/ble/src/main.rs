//! stephania-ble: BlueZ GATT adapter for Stephania provisioning (protocol v3).
//!
//! A thin process with no secrets and no protocol decisions: it publishes
//! one service with two characteristics and forwards request fragments to
//! `stephania-provision` (`gatt.request`, docs/IPC.md framing).
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
//!   active one disconnects or idles out. A central that has not completed
//!   a claim or an owner-authenticated operation (as reported by the device
//!   core) idles out sooner and holds the gate for a bounded time, so a
//!   stranger in radio range cannot keep the owner out. Partial transfers
//!   are dropped on disconnect.
//! - Self-healing: if bluetoothd restarts or the adapter disappears, the
//!   process exits non-zero so systemd restarts it and re-registers the
//!   application and advertisement. A powered-off adapter is powered back
//!   on. The advertisement is re-registered when the controller reports
//!   Powered, when BlueZ drops it, and every 30 s while no central is
//!   connected (guards against "registered but not advertising").
//! - Adapter selection prefers a USB controller (external dongle) over the
//!   on-board UART controller when `STEPHANIA_BLE_ADAPTER=auto`.

use bluer::adv::{Advertisement, AdvertisementHandle, Type as AdvType};
use bluer::gatt::local::{
    Application, Characteristic, CharacteristicNotifier, CharacteristicNotify, CharacteristicNotifyMethod,
    CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, ReqError, Service,
};
use bluer::{Adapter, AdapterEvent, AdapterProperty, Address, DeviceEvent, DeviceProperty, SessionEvent};
use futures::{FutureExt, StreamExt};
use serde_json::{json, Value};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stephania::ipc::{read_frame, write_frame, IO_TIMEOUT};
use stephania::provisioning::{fragment, hex, MIN_ATT_PAYLOAD};
use stephania::provisioning_gatt::{INFO_UUID, INFO_VALUE, RPC_UUID, SERVICE_UUID};

/// An idle authenticated central stops blocking others after this long.
const PEER_IDLE: Duration = Duration::from_secs(30);
/// An unauthenticated central (no claim or owner proof on this link yet)
/// stops blocking others after this long without a completed request...
const UNAUTHENTICATED_IDLE: Duration = Duration::from_secs(10);
/// ...and after this long in total, however busy it keeps itself.
const UNAUTHENTICATED_HOLD: Duration = Duration::from_secs(45);
/// A write still inside the device core (a Wi-Fi join takes ~30 s) keeps
/// its central admitted, up to this long.
const IN_FLIGHT_LIMIT: Duration = Duration::from_secs(90);
const WATCHDOG_PERIOD: Duration = Duration::from_secs(10);
/// While no central is connected, re-register the advertisement this often:
/// BlueZ can report an advertisement as registered while the controller is
/// not actually advertising (e.g. registered around a power transition).
const ADVERTISING_REFRESH: Duration = Duration::from_secs(30);

/// Socket client to stephania-provision; one long-lived connection, reconnected
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

#[derive(Debug, Clone, Copy)]
struct Active {
    peer: Address,
    admitted: Instant,
    /// Last completed request (or admission).
    last: Instant,
    authenticated: bool,
    /// Writes currently being handled by the device core.
    in_flight: u32,
}

impl Active {
    fn expired(&self, now: Instant) -> bool {
        if self.in_flight > 0 && now.duration_since(self.last) < IN_FLIGHT_LIMIT {
            return false;
        }
        if self.authenticated {
            now.duration_since(self.last) >= PEER_IDLE
        } else {
            now.duration_since(self.last) >= UNAUTHENTICATED_IDLE || now.duration_since(self.admitted) >= UNAUTHENTICATED_HOLD
        }
    }
}

/// Which central may currently talk to the device.
#[derive(Debug, Default)]
pub struct PeerGate {
    active: Option<Active>,
}

impl PeerGate {
    /// Admit a write from `peer` if no other central is active (or the
    /// active one expired). Returns the displaced peer when another central
    /// expired. Every admitted write must be followed by [`PeerGate::finish`].
    pub fn admit(&mut self, peer: Address, now: Instant) -> Result<Option<Address>, ReqError> {
        match &mut self.active {
            Some(a) if a.peer == peer => { a.in_flight += 1; Ok(None) }
            Some(a) if !a.expired(now) => Err(ReqError::InProgress),
            _ => {
                let previous = self.active.replace(Active { peer, admitted: now, last: now, authenticated: false, in_flight: 1 });
                Ok(previous.map(|a| a.peer))
            }
        }
    }

    /// A write from `peer` was handled. `completed`: it finished a request
    /// (a response is due); a fragment that never completes a request does
    /// not keep the gate. `authenticated`: the device core reports that this
    /// link completed a claim or an owner-authenticated operation.
    pub fn finish(&mut self, peer: Address, now: Instant, completed: bool, authenticated: bool) {
        if let Some(a) = self.active.as_mut().filter(|a| a.peer == peer) {
            a.in_flight = a.in_flight.saturating_sub(1);
            a.authenticated |= authenticated;
            if completed { a.last = now; }
        }
    }

    pub fn release(&mut self, peer: Address) -> bool {
        if matches!(self.active, Some(a) if a.peer == peer) { self.active = None; true } else { false }
    }

    pub fn active(&self) -> Option<Address> { self.active.map(|a| a.peer) }
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
    {
        let (completed, authenticated) = match &r {
            Ok(v) => (v["response_hex"].is_string(), v["authenticated"] == true),
            Err(_) => (false, false),
        };
        shared.gate.lock().unwrap_or_else(|e| e.into_inner()).finish(peer, Instant::now(), completed, authenticated);
    }
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
            eprintln!("stephania-ble: response dropped, client not subscribed to indications");
            return;
        };
        for f in frags {
            if let Err(e) = n.notify(f).await {
                eprintln!("stephania-ble: indication failed: {e}");
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
    match std::env::var("STEPHANIA_BLE_ADAPTER").unwrap_or_else(|_| "auto".into()).as_str() {
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
        eprintln!("stephania-ble: fatal: {e}");
        std::process::exit(2);
    }
}

async fn run() -> bluer::Result<()> {
    let socket = std::env::var_os("STEPHANIA_PROVISION_SOCKET").map(PathBuf::from)
        .unwrap_or_else(|| "/run/stephania-provision/provisioning.sock".into());
    // Off by default: requiring link encryption forces BLE pairing/bonding,
    // the most common source of interoperability failures. The protocol
    // authenticates and seals at the application layer instead.
    let encrypt = std::env::var("STEPHANIA_BLE_REQUIRE_ENCRYPTION").as_deref() == Ok("1");
    let session = bluer::Session::new().await?;
    let adapter = pick_adapter(&session).await?;
    adapter.set_powered(true).await?;
    let address = adapter.address().await?;
    let name = format!("Stephania-{:02X}{:02X}", address.0[4], address.0[5]);

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
    eprintln!("stephania-ble: advertising {name} on {} (link encryption {})", adapter.name(), if encrypt { "required" } else { "off" });

    let adapter_name = adapter.name().to_owned();
    let mut session_events = Box::pin(session.events().await?);
    let mut adapter_events = Box::pin(adapter.events().await?);
    let mut advertised_at = Instant::now();
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
            ev = adapter_events.next() => {
                if let Some(AdapterEvent::PropertyChanged(AdapterProperty::Powered(true))) = ev {
                    // Controller (re)powered: an advertisement registered
                    // before this may never have started.
                    eprintln!("stephania-ble: adapter powered; re-registering advertisement");
                    drop(adv);
                    adv = advertise(&adapter, &name).await?;
                    advertised_at = Instant::now();
                }
            }
            _ = tick.tick() => {
                // Errors here mean bluetoothd restarted or the adapter vanished.
                if !adapter.is_powered().await? {
                    eprintln!("stephania-ble: adapter powered off; powering on");
                    adapter.set_powered(true).await?;
                    continue;
                }
                let idle = shared.gate.lock().unwrap_or_else(|e| e.into_inner()).active().is_none();
                if !idle { continue; }
                let dropped = adapter.active_advertising_instances().await? == 0;
                if dropped || advertised_at.elapsed() >= ADVERTISING_REFRESH {
                    if dropped { eprintln!("stephania-ble: advertising was dropped by BlueZ; re-registering"); }
                    drop(adv);
                    adv = advertise(&adapter, &name).await?;
                    advertised_at = Instant::now();
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
        let s = Duration::from_secs;
        let mut g = PeerGate::default();
        assert_eq!(g.admit(a, t), Ok(None));
        g.finish(a, t, true, true);
        assert_eq!(g.admit(a, t + s(5)), Ok(None));
        g.finish(a, t + s(5), true, false);
        assert_eq!(g.admit(b, t + s(10)), Err(ReqError::InProgress));
        // a (authenticated) idles out: b takes over and a's partial state must be dropped.
        assert_eq!(g.admit(b, t + s(5) + PEER_IDLE), Ok(Some(a)));
        g.finish(b, t + s(5) + PEER_IDLE, true, false);
        assert!(!g.release(a));
        assert!(g.release(b));
        assert_eq!(g.admit(a, t + s(100)), Ok(None));
    }

    #[test]
    fn unauthenticated_central_cannot_hold_the_gate() {
        let attacker = Address([1, 1, 1, 1, 1, 1]);
        let owner = Address([2, 2, 2, 2, 2, 2]);
        let t = Instant::now();
        let s = Duration::from_secs;
        let mut g = PeerGate::default();
        // Fragments that never complete a request do not keep the gate.
        assert_eq!(g.admit(attacker, t), Ok(None));
        g.finish(attacker, t, false, false);
        for i in 1..5 {
            assert_eq!(g.admit(attacker, t + s(2 * i)), Ok(None));
            g.finish(attacker, t + s(2 * i), false, false);
        }
        assert_eq!(g.admit(owner, t + UNAUTHENTICATED_IDLE - s(1)), Err(ReqError::InProgress));
        assert_eq!(g.admit(owner, t + UNAUTHENTICATED_IDLE), Ok(Some(attacker)));
        g.finish(owner, t + UNAUTHENTICATED_IDLE, true, false);
        assert!(g.release(owner));
        // Completed but unauthenticated requests hold it for a bounded time.
        let t = t + s(100);
        assert_eq!(g.admit(attacker, t), Ok(None));
        g.finish(attacker, t, true, false);
        let mut now = t;
        while now < t + UNAUTHENTICATED_HOLD {
            assert_eq!(g.admit(owner, now), Err(ReqError::InProgress));
            now += s(5);
            assert_eq!(g.admit(attacker, now), Ok(None));
            g.finish(attacker, now, true, false);
        }
        assert_eq!(g.admit(owner, now), Ok(Some(attacker)));
        g.finish(owner, now, true, false);
        // A request still inside the device core (a Wi-Fi join) is never cut off.
        assert_eq!(g.admit(owner, now + s(1)), Ok(None));
        g.finish(owner, now + s(1), true, false);
        assert_eq!(g.admit(owner, now + s(2)), Ok(None));
        assert_eq!(g.admit(attacker, now + s(2) + UNAUTHENTICATED_HOLD), Err(ReqError::InProgress));
        g.finish(owner, now + s(2) + UNAUTHENTICATED_HOLD, true, true);
        // Authenticated: no hold limit, the normal idle time.
        let later = now + s(2) + UNAUTHENTICATED_HOLD;
        assert_eq!(g.admit(attacker, later + PEER_IDLE - s(1)), Err(ReqError::InProgress));
        assert_eq!(g.admit(attacker, later + PEER_IDLE), Ok(Some(owner)));
    }

    #[test]
    fn info_value_never_needs_a_long_read() {
        assert!(INFO_VALUE.len() <= 22);
    }
}
