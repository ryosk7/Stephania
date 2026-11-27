//! T3Box independent media components.
pub mod flv;
pub mod rtmp;
pub mod handshake;
pub mod amf0;
pub mod amf0_decode;
pub mod rtmp_read;
pub mod session;
pub mod rtmps_tls;
#[cfg(target_os = "linux")]
pub mod tcp_telemetry;
pub mod media_output;
pub mod flv_input;
pub mod flv_relay;
pub mod media_core;
pub mod scene;
pub mod ipc;
pub mod compositor;
pub mod provisioning;
pub mod provisioning_gatt;
#[cfg(unix)]
pub mod nm;
#[cfg(all(unix, feature = "networkmanager"))]
pub mod nm_dbus;

#[cfg(all(target_os = "linux", feature = "native-capture"))]
pub mod native_capture;

pub mod colorspace;
pub mod content_analysis;

#[cfg(feature = "native-x264")]
pub mod native_x264;

#[cfg(all(feature = "native-capture", feature = "native-x264"))]
pub mod native_video;

pub mod pcm_framer;

pub mod aac_adts;

pub mod av_clock;
pub mod reconnect;
pub mod rate_control;
pub mod transport_deadline;

#[cfg(feature = "native-aac")]
pub mod native_aac;

#[cfg(test)]
mod accelerated_ci;

#[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
pub mod native_av;
#[cfg(all(target_os = "linux", feature = "native-capture", feature = "native-x264", feature = "native-aac"))]
mod media_worker;
