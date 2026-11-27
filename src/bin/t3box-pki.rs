//! Device PKI maintenance (crate::pki).
//!
//! `t3box-pki ensure [--host <label>] [--port <n>] [--ip <addr>]... [--key-group <group>]`
//!
//! Creates the device CA on first use and (re)issues the LAN TLS server
//! certificate when the host name, addresses or remaining validity require
//! it. Prints `{"result":"issued"|"unchanged","endpoint":{…}}`. Exit status
//! is 0 when unchanged and 10 when a certificate was issued, so a systemd
//! unit can restart the TLS server only then. Without `--ip`, addresses of
//! up, non-loopback interfaces in private/link-local ranges are used.
//! `--key-group` makes the server key readable by that group (the control
//! plane); the CA key stays owner-only.
use std::net::IpAddr;
use t3box::pki;

fn usage() -> ! {
    eprintln!("usage: t3box-pki ensure [--host <label>] [--port <n>] [--ip <addr>]... [--key-group <group>]");
    std::process::exit(2);
}

fn default_host() -> String {
    std::env::var("T3BOX_HOSTNAME").ok().filter(|h| !h.is_empty())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_lowercase()))
        .unwrap_or_else(|| "t3box".into())
}

#[cfg(unix)]
fn chgrp(path: &std::path::Path, group: &str) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let g = CString::new(group).map_err(|_| "bad group".to_string())?;
    let p = CString::new(path.as_os_str().as_bytes()).map_err(|_| "bad path".to_string())?;
    // SAFETY: valid NUL-terminated strings; getgrnam's result is read
    // immediately and not retained.
    unsafe {
        let gr = libc::getgrnam(g.as_ptr());
        if gr.is_null() { return Err(format!("unknown group {group}")); }
        if libc::chown(p.as_ptr(), u32::MAX, (*gr).gr_gid) != 0 {
            return Err(format!("chgrp {}: {}", path.display(), std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("ensure") { usage(); }
    let (mut host, mut port, mut ips, mut group) = (default_host(), 9443u16, Vec::<IpAddr>::new(), None::<String>);
    while let Some(a) = args.next() {
        let v = args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--host" => host = v,
            "--port" => port = v.parse().unwrap_or_else(|_| usage()),
            "--ip" => ips.push(v.parse().unwrap_or_else(|_| usage())),
            "--key-group" => group = Some(v),
            _ => usage(),
        }
    }
    #[cfg(unix)]
    if ips.is_empty() { ips = pki::interface_addresses(); }
    let dir = pki::default_dir();
    match pki::ensure(&dir, &host, &ips, port, std::time::SystemTime::now()) {
        Ok((result, endpoint)) => {
            #[cfg(unix)]
            if let Some(g) = &group {
                if let Err(e) = chgrp(&dir.join("server-key.pem"), g) {
                    eprintln!("t3box-pki: {e}");
                    std::process::exit(1);
                }
            }
            let issued = result == pki::Ensure::Issued;
            println!("{}", serde_json::json!({"result": if issued {"issued"} else {"unchanged"}, "endpoint": endpoint.to_json()}));
            std::process::exit(if issued { 10 } else { 0 });
        }
        Err(e) => {
            eprintln!("t3box-pki: {e}");
            std::process::exit(1);
        }
    }
}
