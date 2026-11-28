//! stephania-phone: WebRTC receiver for the phone's camera tracks (docs/PHONE.md).
//!
//! - Control: Unix socket (`STEPHANIA_PHONE_SOCKET`, docs/IPC.md framing),
//!   used only by the control plane, which relays the phone's SDP:
//!   `session.offer {sdp, tracks:{<track id>: "rear"|"front"}, replace?}` →
//!   `{sdp}` (non-trickle answer); `session.close`; `session.status`.
//! - Media: ICE host candidates on one UDP port (`STEPHANIA_PHONE_UDP_PORT`,
//!   default 40100) on the device's private addresses, re-read for every
//!   offer so a session follows Wi-Fi provisioning or a DHCP renumbering
//!   without a restart; DTLS-SRTP; H.264
//!   only. RTP is reassembled into access units (assembler.rs) and
//!   forwarded to the engine's ingest socket (`STEPHANIA_MEDIA_SOCKET`,
//!   crate `stephania::phone_ingest`), which decodes them. Keyframe requests
//!   from the engine, and losses seen here, become RTCP PLI.
//! - Exactly one phone session (§12). A second offer is refused with
//!   `busy` unless `replace:true`.
//!
//! This process holds no device secrets; it can crash or be restarted
//! without affecting the engine.
mod assembler;

use assembler::{Assembler, Output};
use rtc::interceptor::Registry;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::{MediaEngine, MIME_TYPE_H264};
use rtc::peer_connection::configuration::RTCConfigurationBuilder;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters, RtpCodecKind};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, oneshot};
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState, RTCPeerConnectionState};

const MAX_FRAME: usize = 64 * 1024;
const PLI_INTERVAL: Duration = Duration::from_millis(500);
const GATHER_TIMEOUT: Duration = Duration::from_secs(5);
const ENGINE_QUEUE: usize = 32;

fn env(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_owned())
}

fn source_id(name: &str) -> Option<u8> {
    match name { "rear" => Some(1), "front" => Some(2), _ => None }
}

/// Live per-session state reported by `session.status`.
#[derive(Default)]
struct Stats {
    state: String,
    access_units: HashMap<&'static str, u64>,
    keyframe_requests: u64,
    dropped: u64,
    since: Option<Instant>,
}

struct Session {
    pc: Arc<dyn PeerConnection>,
    stats: Arc<Mutex<Stats>>,
}

struct Shared {
    session: tokio::sync::Mutex<Option<Session>>,
    to_engine: mpsc::Sender<Vec<u8>>,
    keyframe_requests: broadcast::Sender<u8>,
}

#[derive(Clone)]
struct Handler {
    gathered: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    tracks: Arc<HashMap<String, u8>>,
    stats: Arc<Mutex<Stats>>,
    to_engine: mpsc::Sender<Vec<u8>>,
    keyframe_requests: broadcast::Sender<u8>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            if let Some(tx) = self.gathered.lock().unwrap().take() { let _ = tx.send(()); }
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        eprintln!("stephania-phone: connection {state}");
        self.stats.lock().unwrap().state = state.to_string();
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        if track.kind().await != RtpCodecKind::Video { return; }
        let id = track.track_id().await.to_string();
        let Some(&source) = self.tracks.get(&id) else {
            eprintln!("stephania-phone: ignoring unmapped track {id}");
            return;
        };
        let name = stephania_source_name(source);
        let Some(&ssrc) = track.ssrcs().await.first() else { return };
        eprintln!("stephania-phone: track {id} → {name} (ssrc {ssrc})");
        let (to_engine, stats) = (self.to_engine.clone(), Arc::clone(&self.stats));
        let mut engine_requests = self.keyframe_requests.subscribe();
        tokio::spawn(async move {
            let mut assembler = Assembler::default();
            let mut last_pli: Option<Instant> = None;
            pli(&mut last_pli, Arc::clone(&track), Arc::clone(&stats), ssrc).await;
            loop {
                tokio::select! {
                    event = track.poll() => {
                        let Some(event) = event else { break };
                        let TrackRemoteEvent::OnRtpPacket(p) = event else {
                            if matches!(event, TrackRemoteEvent::OnEnded) { break; }
                            continue;
                        };
                        for out in assembler.push(p.header.sequence_number, p.header.timestamp, p.header.marker, &p.payload) {
                            match out {
                                Output::NeedKeyframe => pli(&mut last_pli, Arc::clone(&track), Arc::clone(&stats), ssrc).await,
                                Output::AccessUnit { timestamp, data, .. } => {
                                    // 90 kHz RTP clock → microseconds (wraps every ~13 h;
                                    // the engine only needs monotonic order per source).
                                    let pts_us = u64::from(timestamp) * 100 / 9;
                                    match to_engine.try_send(stephania_access_unit(source, pts_us, &data)) {
                                        Ok(()) => *stats.lock().unwrap().access_units.entry(name).or_default() += 1,
                                        Err(_) => {
                                            // Engine slow or absent: drop and resynchronise
                                            // on a keyframe rather than queue latency.
                                            stats.lock().unwrap().dropped += 1;
                                            assembler = Assembler::default();
                                            pli(&mut last_pli, Arc::clone(&track), Arc::clone(&stats), ssrc).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    req = engine_requests.recv() => {
                        if matches!(req, Ok(s) if s == source) {
                            pli(&mut last_pli, Arc::clone(&track), Arc::clone(&stats), ssrc).await;
                        }
                    }
                }
            }
            eprintln!("stephania-phone: track {name} ended");
        });
    }
}

/// Sends an RTCP PLI unless one went out within PLI_INTERVAL.
async fn pli(last: &mut Option<Instant>, track: Arc<dyn TrackRemote>, stats: Arc<Mutex<Stats>>, ssrc: u32) {
    if last.is_some_and(|t| t.elapsed() < PLI_INTERVAL) { return; }
    *last = Some(Instant::now());
    stats.lock().unwrap().keyframe_requests += 1;
    let _ = track.write_rtcp(vec![Box::new(PictureLossIndication { sender_ssrc: 0, media_ssrc: ssrc })]).await;
}

fn stephania_source_name(id: u8) -> &'static str { if id == 1 { "phone:rear" } else { "phone:front" } }

fn stephania_access_unit(source: u8, pts_us: u64, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + data.len());
    out.extend_from_slice(&((10 + data.len()) as u32).to_be_bytes());
    out.push(1);
    out.push(source);
    out.extend_from_slice(&pts_us.to_be_bytes());
    out.extend_from_slice(data);
    out
}

fn parse_tracks(v: Option<&Value>) -> Result<HashMap<String, u8>, &'static str> {
    let map = v.and_then(Value::as_object).ok_or("tracks must map track ids to rear|front")?;
    if map.is_empty() || map.len() > 2 { return Err("one or two tracks"); }
    let mut out = HashMap::new();
    for (id, role) in map {
        if id.is_empty() || id.len() > 128 { return Err("invalid track id"); }
        let s = role.as_str().and_then(source_id).ok_or("track role must be rear or front")?;
        if out.values().any(|&v| v == s) { return Err("duplicate track role"); }
        out.insert(id.clone(), s);
    }
    Ok(out)
}

async fn new_session(shared: &Shared, offer_sdp: String, tracks: HashMap<String, u8>) -> Result<(Session, String), String> {
    let mut media = MediaEngine::default();
    media.register_codec(RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: 102,
    }, RtpCodecKind::Video).map_err(|e| e.to_string())?;
    let registry = register_default_interceptors(Registry::new(), &mut media).map_err(|e| e.to_string())?;
    // Read now, not at startup: the address set changes after BLE Wi-Fi
    // provisioning, a network switch or a new DHCP lease.
    let udp_addrs = udp_addrs();
    if udp_addrs.is_empty() { return Err("no private IPv4 address; the network is not up".into()); }
    let stats = Arc::new(Mutex::new(Stats { state: "new".into(), since: Some(Instant::now()), ..Default::default() }));
    let (gather_tx, gather_rx) = oneshot::channel();
    let handler = Arc::new(Handler {
        gathered: Arc::new(Mutex::new(Some(gather_tx))),
        tracks: Arc::new(tracks),
        stats: Arc::clone(&stats),
        to_engine: shared.to_engine.clone(),
        keyframe_requests: shared.keyframe_requests.clone(),
    });
    // LAN only: no STUN/TURN servers; host candidates on configured addresses.
    let pc = PeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(handler as Arc<dyn PeerConnectionEventHandler>)
        .with_udp_addrs(udp_addrs)
        .build()
        .await
        .map_err(|e| format!("peer connection: {e}"))?;
    let pc: Arc<dyn PeerConnection> = Arc::new(pc);
    let result = async {
        let offer = RTCSessionDescription::offer(offer_sdp).map_err(|e| format!("invalid offer: {e}"))?;
        pc.set_remote_description(offer).await.map_err(|e| format!("invalid offer: {e}"))?;
        let answer = pc.create_answer(None).await.map_err(|e| format!("answer: {e}"))?;
        pc.set_local_description(answer).await.map_err(|e| format!("answer: {e}"))?;
        tokio::time::timeout(GATHER_TIMEOUT, gather_rx).await.map_err(|_| "ICE gathering timed out".to_string())?.ok();
        pc.local_description().await.map(|d| d.sdp).ok_or_else(|| "no local description".to_string())
    }.await;
    match result {
        Ok(sdp) => Ok((Session { pc, stats }, sdp)),
        Err(e) => { let _ = pc.close().await; Err(e) }
    }
}

async fn handle(shared: &Shared, method: &str, params: &Map<String, Value>) -> Result<Value, (&'static str, String)> {
    match method {
        "session.offer" => {
            let sdp = params.get("sdp").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 32 * 1024)
                .ok_or(("invalid_params", "sdp required".to_string()))?;
            let tracks = parse_tracks(params.get("tracks")).map_err(|e| ("invalid_params", e.to_string()))?;
            let replace = params.get("replace").and_then(Value::as_bool).unwrap_or(false);
            let mut slot = shared.session.lock().await;
            if let Some(existing) = slot.as_ref() {
                let state = existing.stats.lock().unwrap().state.clone();
                let alive = !matches!(state.as_str(), "failed" | "closed" | "disconnected");
                if alive && !replace { return Err(("busy", "a phone session is active".into())); }
            }
            if let Some(old) = slot.take() { let _ = old.pc.close().await; }
            let (session, answer) = new_session(shared, sdp.to_owned(), tracks).await.map_err(|e| ("invalid_offer", e))?;
            *slot = Some(session);
            Ok(json!({"sdp": answer}))
        }
        "session.close" => {
            if let Some(old) = shared.session.lock().await.take() { let _ = old.pc.close().await; }
            Ok(json!({}))
        }
        "session.status" => {
            let slot = shared.session.lock().await;
            Ok(match slot.as_ref() {
                None => json!({"state": "none"}),
                Some(s) => {
                    let st = s.stats.lock().unwrap();
                    json!({"state": st.state, "access_units": st.access_units, "keyframe_requests": st.keyframe_requests,
                           "dropped": st.dropped, "age_ms": st.since.map(|t| t.elapsed().as_millis() as u64)})
                }
            })
        }
        _ => Err(("unknown_method", "unknown method".into())),
    }
}

async fn serve_control(stream: &mut UnixStream, shared: &Shared) -> std::io::Result<()> {
    loop {
        let mut len = [0u8; 4];
        if stream.read_exact(&mut len).await.is_err() { return Ok(()); }
        let n = u32::from_be_bytes(len) as usize;
        if n == 0 || n > MAX_FRAME { return Ok(()); }
        let mut body = vec![0u8; n];
        stream.read_exact(&mut body).await?;
        let reply = match serde_json::from_slice::<Value>(&body) {
            Ok(Value::Object(req)) if req.get("version") == Some(&json!(1)) => {
                let id = req.get("request_id").cloned().unwrap_or(Value::Null);
                let empty = Map::new();
                let params = req.get("params").and_then(Value::as_object).unwrap_or(&empty);
                match handle(shared, req.get("method").and_then(Value::as_str).unwrap_or(""), params).await {
                    Ok(result) => json!({"version": 1, "request_id": id, "ok": true, "result": result}),
                    Err((code, message)) => json!({"version": 1, "request_id": id, "ok": false, "error": {"code": code, "message": message}}),
                }
            }
            _ => json!({"version": 1, "request_id": null, "ok": false, "error": {"code": "invalid_request", "message": "invalid frame"}}),
        };
        let out = reply.to_string().into_bytes();
        stream.write_all(&(out.len() as u32).to_be_bytes()).await?;
        stream.write_all(&out).await?;
    }
}

/// Keeps one connection to the engine ingest socket; reconnects forever.
async fn engine_link(path: String, mut rx: mpsc::Receiver<Vec<u8>>, keyframe_requests: broadcast::Sender<u8>) {
    loop {
        match UnixStream::connect(&path).await {
            Ok(stream) => {
                eprintln!("stephania-phone: connected to engine ingest {path}");
                let (mut r, mut w) = stream.into_split();
                let kf = keyframe_requests.clone();
                let reader = tokio::spawn(async move {
                    let mut msg = [0u8; 6];
                    while r.read_exact(&mut msg).await.is_ok() {
                        if msg[..5] == [0, 0, 0, 2, 2] { let _ = kf.send(msg[5]); }
                    }
                });
                // Ask every track for a keyframe: the engine's decoders are new.
                for s in [1u8, 2] { let _ = keyframe_requests.send(s); }
                while let Some(frame) = rx.recv().await {
                    if w.write_all(&frame).await.is_err() { break; }
                }
                reader.abort();
            }
            Err(_) => {
                // Engine absent: discard queued media instead of building latency.
                while rx.try_recv().is_ok() {}
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// ICE bind addresses: `STEPHANIA_PHONE_ADDRS` if set, otherwise the
/// device's current private IPv4 addresses.
fn udp_addrs() -> Vec<String> {
    let port = env("STEPHANIA_PHONE_UDP_PORT", "40100");
    let fixed = std::env::var("STEPHANIA_PHONE_ADDRS").ok();
    select_udp_addrs(fixed.as_deref(), stephania::pki::interface_addresses(), &port)
}

fn select_udp_addrs(fixed: Option<&str>, interfaces: Vec<IpAddr>, port: &str) -> Vec<String> {
    if let Some(list) = fixed {
        return list.split(',').filter(|a| !a.is_empty()).map(|a| format!("{a}:{port}")).collect();
    }
    let mut ips: Vec<IpAddr> = interfaces.into_iter()
        .filter(|ip| ip.is_ipv4() && stephania::pki::certifiable(ip) && !ip.is_loopback()).collect();
    ips.sort();
    ips.dedup();
    ips.into_iter().map(|ip| format!("{ip}:{port}")).collect()
}

#[tokio::main]
async fn main() {
    let socket = env("STEPHANIA_PHONE_SOCKET", "/run/stephania-phone/phone.sock");
    let media = env("STEPHANIA_MEDIA_SOCKET", "/run/stephania/media.sock");
    let addrs = udp_addrs();
    if addrs.is_empty() {
        eprintln!("stephania-phone: no private IPv4 address yet; offers are refused until the network is up");
    }
    let (to_engine, rx) = mpsc::channel(ENGINE_QUEUE);
    let (keyframe_requests, _) = broadcast::channel(16);
    tokio::spawn(engine_link(media, rx, keyframe_requests.clone()));
    let shared = Arc::new(Shared { session: tokio::sync::Mutex::new(None), to_engine, keyframe_requests });
    let _ = std::fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(e) => { eprintln!("stephania-phone: bind {socket}: {e}"); std::process::exit(2); }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660));
    }
    eprintln!("stephania-phone: control {socket}, media UDP now {addrs:?} (re-read per offer)");
    loop {
        let Ok((mut stream, _)) = listener.accept().await else { continue };
        let shared = Arc::clone(&shared);
        tokio::spawn(async move { let _ = serve_control(&mut stream, &shared).await; });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_addresses_follow_the_current_interfaces() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(select_udp_addrs(None, vec![ip("127.0.0.1")], "40100").is_empty(), "loopback only: not up yet");
        assert_eq!(select_udp_addrs(None, vec![ip("192.168.1.20"), ip("127.0.0.1"), ip("192.168.1.20")], "40100"),
                   vec!["192.168.1.20:40100"]);
        assert!(select_udp_addrs(None, vec![ip("8.8.8.8"), ip("fd00::1")], "40100").is_empty(), "public and IPv6 skipped");
        assert_eq!(select_udp_addrs(Some("10.0.0.5,"), vec![ip("192.168.1.20")], "5000"), vec!["10.0.0.5:5000"]);
    }
}
