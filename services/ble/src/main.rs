//! t3box-ble: BlueZ GATT adapter for T3Box provisioning.
//!
//! A thin process that publishes the provisioning service over BLE and
//! forwards every characteristic read/write to `t3box-provision` over its
//! Unix socket (`gatt.read` / `gatt.write`, docs/IPC.md framing). It holds no
//! secrets and makes no decisions; the protocol lives in the device core.
//!
//! Contract details implemented here (contracts/ble-protocol.md):
//! - Reads: the value is fetched once at offset 0 and cached per
//!   (device, characteristic) so ATT long reads (offset > 0) return slices of
//!   the same value. A fresh offset-0 read refreshes it. This matters for
//!   PairingChallenge, whose read has a side effect.
//! - Writes: each fragment is forwarded; when the device core returns a
//!   response (or an error), it is notified on the same characteristic,
//!   fragmented to the client's ATT payload under the request's message id.

use bluer::adv::{Advertisement, Type as AdvType};
use bluer::gatt::local::{
    Application, Characteristic, CharacteristicNotifier, CharacteristicNotify, CharacteristicNotifyMethod,
    CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, ReqError, Service,
};
use bluer::Address;
use futures::FutureExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use t3box::ipc::{read_frame, write_frame, IO_TIMEOUT};
use t3box::provisioning::{fragment, hex};
use t3box::provisioning_gatt::{Characteristic as Char, SERVICE_UUID};
use uuid::Uuid;

/// Socket client to t3box-provision. Serialised: the device core handles one
/// request at a time anyway, and BLE provisioning is low rate.
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
                let _ = s.set_read_timeout(Some(IO_TIMEOUT * 10)); // Wi-Fi join may take ~30 s
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
                // Stale connection (provision restarted): reconnect once.
                Err(_) if attempt == 0 => { self.stream = None; }
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

/// ATT payload for notifications: MTU − 3, within protocol bounds.
pub fn att_payload(mtu: u16) -> usize {
    (mtu as usize).saturating_sub(3).clamp(t3box::provisioning::MIN_ATT_PAYLOAD, 509)
}

/// Slice a cached long-read value at `offset`.
pub fn slice_at(value: &[u8], offset: u16) -> Result<Vec<u8>, ReqError> {
    let offset = offset as usize;
    if offset > value.len() { Err(ReqError::InvalidOffset) } else { Ok(value[offset..].to_vec()) }
}

type Notifiers = Arc<tokio::sync::Mutex<HashMap<Uuid, CharacteristicNotifier>>>;

struct Shared {
    provision: Mutex<Provision>,
    read_cache: Mutex<HashMap<(Address, Uuid), Vec<u8>>>,
    notifiers: Notifiers,
}

async fn provision_call(shared: &Arc<Shared>, method: &'static str, params: Value) -> Result<Value, String> {
    let shared = Arc::clone(shared);
    tokio::task::spawn_blocking(move || shared.provision.lock().unwrap_or_else(|e| e.into_inner()).call(method, params))
        .await
        .map_err(|_| "internal_error".to_string())?
}

fn characteristic(c: Char, shared: &Arc<Shared>, encrypt: bool) -> Characteristic {
    let uuid: Uuid = c.uuid().parse().expect("valid uuid");
    let mut ch = Characteristic { uuid, ..Default::default() };
    if !c.writable() {
        let shared = Arc::clone(shared);
        ch.read = Some(CharacteristicRead {
            read: true,
            encrypt_read: encrypt,
            fun: Box::new(move |req| {
                let shared = Arc::clone(&shared);
                async move {
                    let key = (req.device_address, uuid);
                    if req.offset > 0 {
                        let cache = shared.read_cache.lock().unwrap_or_else(|e| e.into_inner());
                        return cache.get(&key).map_or(Err(ReqError::InvalidOffset), |v| slice_at(v, req.offset));
                    }
                    let r = provision_call(&shared, "gatt.read", json!({"uuid": c.uuid()})).await;
                    let value = match r {
                        Ok(v) => v["value_hex"].as_str().and_then(unhex).ok_or(ReqError::Failed)?,
                        Err(code) if code == "not_owner" || code == "forbidden" => return Err(ReqError::NotAuthorized),
                        Err(code) if code.starts_with("provision_unavailable") => return Err(ReqError::Failed),
                        Err(_) => return Err(ReqError::NotPermitted),
                    };
                    shared.read_cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, value.clone());
                    Ok(value)
                }
                .boxed()
            }),
            ..Default::default()
        });
        return ch;
    }
    let write_shared = Arc::clone(shared);
    ch.write = Some(CharacteristicWrite {
        write: true,
        encrypt_write: encrypt,
        method: CharacteristicWriteMethod::Fun(Box::new(move |value, req| {
            let shared = Arc::clone(&write_shared);
            async move {
                if value.is_empty() || req.offset != 0 { return Err(ReqError::InvalidValueLength); }
                let message_id = value[0];
                let r = provision_call(&shared, "gatt.write", json!({"uuid": c.uuid(), "fragment_hex": hex(&value)})).await;
                let response = match r {
                    Ok(v) => match v["response_hex"].as_str() {
                        Some(h) => unhex(h).ok_or(ReqError::Failed)?,
                        None => return Ok(()), // more fragments expected
                    },
                    Err(code) if code.starts_with("provision_unavailable") => return Err(ReqError::Failed),
                    // Contract: device errors are notified as {"error": code}.
                    Err(code) => serde_json::to_vec(&json!({"error": code})).map_err(|_| ReqError::Failed)?,
                };
                let att = att_payload(req.mtu);
                let notifiers = Arc::clone(&shared.notifiers);
                // Notify after the write is acknowledged.
                tokio::spawn(async move {
                    let Ok(frags) = fragment(message_id, &response, att) else { return };
                    let mut map = notifiers.lock().await;
                    let Some(n) = map.get_mut(&uuid) else {
                        eprintln!("t3box-ble: response for {uuid} dropped: client not subscribed");
                        return;
                    };
                    for f in frags {
                        if n.notify(f).await.is_err() { map.remove(&uuid); return; }
                    }
                });
                Ok(())
            }
            .boxed()
        })),
        ..Default::default()
    });
    let notifiers = Arc::clone(&shared.notifiers);
    ch.notify = Some(CharacteristicNotify {
        notify: true,
        method: CharacteristicNotifyMethod::Fun(Box::new(move |notifier| {
            let notifiers = Arc::clone(&notifiers);
            async move { notifiers.lock().await.insert(uuid, notifier); }.boxed()
        })),
        ..Default::default()
    });
    ch
}

#[tokio::main]
async fn main() -> bluer::Result<()> {
    let socket = std::env::var_os("T3BOX_PROVISION_SOCKET").map(PathBuf::from)
        .unwrap_or_else(|| "/run/t3box-provision/provisioning.sock".into());
    // LE link encryption on by default; the app-layer HMAC proofs do not
    // depend on it, it only hides traffic. Can be disabled for test benches.
    let encrypt = std::env::var("T3BOX_BLE_REQUIRE_ENCRYPTION").as_deref() != Ok("0");
    let session = bluer::Session::new().await?;
    let adapter = match std::env::var("T3BOX_BLE_ADAPTER") {
        Ok(name) if !name.is_empty() => session.adapter(&name)?,
        _ => session.default_adapter().await?,
    };
    adapter.set_powered(true).await?;
    let address = adapter.address().await?;
    let name = format!("T3Box-{:02X}{:02X}", address.0[4], address.0[5]);
    let _ = adapter.set_alias(name.clone()).await;

    let shared = Arc::new(Shared {
        provision: Mutex::new(Provision { path: socket, stream: None, next: 0 }),
        read_cache: Mutex::new(HashMap::new()),
        notifiers: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
    });
    let service_uuid: Uuid = SERVICE_UUID.parse().expect("valid uuid");
    let app = Application {
        services: vec![Service {
            uuid: service_uuid,
            primary: true,
            characteristics: Char::ALL.into_iter().map(|c| characteristic(c, &shared, encrypt)).collect(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let _app = adapter.serve_gatt_application(app).await?;
    let _adv = adapter.advertise(Advertisement {
        advertisement_type: AdvType::Peripheral,
        service_uuids: [service_uuid].into_iter().collect(),
        local_name: Some(name.clone()),
        discoverable: Some(true),
        ..Default::default()
    }).await?;
    eprintln!("t3box-ble: advertising {name} on {} (encryption {})", adapter.name(), if encrypt { "required" } else { "off" });

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_read_slicing() {
        let v: Vec<u8> = (0..300u16).map(|i| i as u8).collect();
        assert_eq!(slice_at(&v, 0).unwrap(), v);
        assert_eq!(slice_at(&v, 299).unwrap(), vec![43]);
        assert_eq!(slice_at(&v, 300).unwrap(), Vec::<u8>::new());
        assert!(matches!(slice_at(&v, 301), Err(ReqError::InvalidOffset)));
    }

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
}
