# Maintenance shell access (design)

Status: design, not implemented. Nothing here ships in the image yet.

The owner sometimes needs a shell on the device: read logs, restart a
service, look at the network, collect evidence. Stephania gives that as
**SSH with short-lived certificates**, opened from a laptop with the
`stephctl` CLI and confirmed with a 6-digit code shown in Stephania Connect.
Nobody creates, copies or registers an SSH key; nothing listens for SSH
while no maintenance window is open.

The model follows Teleport (short-lived OpenSSH certificates from a CA,
host certificates instead of trust-on-first-use, a second factor per
access, an audit trail), cut down to one device on a LAN with no proxy.

## Goals

- No long-lived user keys anywhere: every login uses a fresh key pair and
  a certificate valid for minutes.
- Two independent checks before a certificate is signed: a Studio admin
  account (something you know) and a code from the owner's phone
  (something you have).
- The SSH port is closed unless a window is open; it only accepts LAN
  addresses.
- Every window and certificate is attributable (who, from where, when).
- A factory reset revokes everything.

## Non-goals (for now)

- Access from outside the LAN, relays or tunnels.
- Root by default (see [Privileges](#privileges)).
- A browser terminal. The same window and certificate API could back one
  later.
- Android Connect (paused) and Windows-specific polish beyond what the
  OpenSSH client in Windows already provides.

## Overview

```text
laptop: stephctl                         device
────────────────                         ──────
login ── HTTPS (pinned CA) ────────────▶ control: Studio account (admin)
ssh:
  code from Connect ───────────────────▶ control ──unix──▶ stephania-maint
                                          verifies code, opens a 10 min window,
                                          starts stephania-sshd (LAN only)
  fresh Ed25519 key (memory/temp)
  public key ──────────────────────────▶ control ──unix──▶ stephania-maint
  ◀───────────── 5 min user certificate + host CA public key
  ssh -o CertificateFile … ────────────▶ stephania-sshd (port 22)
                                          trusts the user CA; presents a host
                                          certificate, so no host-key prompt
```

## Components

### stephania-maint (new, device)

A small Rust binary (`src/bin/stephania-maint.rs`) running as its own
system user `stephania-maint`. It owns everything that can grant shell
access, so that a compromise of the control plane alone cannot.

- State in `/var/lib/stephania/maint` (0700): the SSH **user CA** and
  **host CA** (Ed25519), the sshd host key and its host certificate, the
  OTP secret, the KRL, and the window state. Created on first boot and
  on factory reset, like the TLS device CA (`stephania-pki`).
- Unix socket `/run/stephania-maint/maint.sock`, group `stephania-control`
  (0660), JSON requests like the engine IPC (`contracts/` gets a schema).
- Starts and stops `stephania-sshd.service` through systemd D-Bus. A polkit
  rule (like `systemd/stephania-provision.polkit.rules`) lets the
  `stephania-maint` user manage that one unit and nothing else.

Socket operations:

| Operation | Request | Result |
|---|---|---|
| `window.open` | `{actor, client_addr, code}` | `{window_id, expires_at}`; starts sshd |
| `cert.sign` | `{window_id, public_key, client_addr}` | `{certificate, valid_before, host_ca, principal, port}` |
| `window.close` | `{window_id}` | `{}`; stops sshd and ends its sessions |
| `status` | `{}` | `{open, expires_at, sessions}` |

Rules enforced here, not in control:

- **Code check.** TOTP (RFC 6238, SHA-1, 6 digits, 30 s step), accepting
  the current step and one step either side. A step that has been used
  once is rejected afterwards (replay). Five wrong codes lock code entry
  for 60 s, doubling to 15 min, the same curve as Studio sign-in.
- **Window.** One window at a time, 10 minutes to start sessions, closed
  early by `window.close`. Sessions started in the window may run to an
  absolute limit of 60 minutes; closing the window or reaching the limit
  ends them (`KillMode=control-group`).
- **Certificate.** Signed with the user CA: principal `maint`, validity
  from now − 30 s to now + 5 min, key ID `"<actor>@<window_id>"`, serial
  from a counter, `source-address=<client_addr>/32`, and only the
  `permit-pty` extension (no agent, port or X11 forwarding, no user rc).
  Only Ed25519 keys are accepted.
- **Audit.** Every request and result goes to the journal with actor,
  client address, window ID and certificate serial. Control mirrors the
  events into its own event log so the Studio can show them.

### stephania-sshd.service (new, device)

The Debian OpenSSH server with its own config, never enabled at boot.

```text
# /etc/stephania/sshd_config (in the image)
Port 22
HostKey /var/lib/stephania/maint/ssh_host_ed25519_key
HostCertificate /var/lib/stephania/maint/ssh_host_ed25519_key-cert.pub
TrustedUserCAKeys /var/lib/stephania/maint/user_ca.pub
RevokedKeys /var/lib/stephania/maint/revoked.krl
AuthorizedPrincipalsFile /etc/stephania/ssh_principals/%u   # "maint" for user maint
AuthorizedKeysFile none
AuthenticationMethods publickey
PubkeyAcceptedAlgorithms ssh-ed25519-cert-v01@openssh.com
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
AllowUsers maint
DisableForwarding yes
PermitUserRC no
MaxAuthTries 2
LoginGraceTime 20
ClientAliveInterval 30
ClientAliveCountMax 4
```

The host certificate is signed by the host CA for the names the TLS
certificate already covers (`<host>.local` and the current LAN addresses)
and is reissued when they change, alongside `stephania-pki`.

**Firewall.** The image has no packet filter today; port 9443 relies on
the control plane's own host checks. sshd needs one: an nftables table
loaded at boot that accepts TCP 22 only from private and link-local
ranges (`10/8`, `172.16/12`, `192.168/16`, `169.254/16`, `fc00::/7`,
`fe80::/10`) and drops it otherwise. It is part of this work.

### Privileges

The `maint` user is not root and has no `sudo`.

- Member of `systemd-journal` (read all logs).
- polkit allows it to restart and stop the `stephania*.service` units and
  read `rauc status`.
- Read access to `/var/lib/stephania` except key material.

Root access is an open question: if needed, add a second principal
`maint-root` that the window must request explicitly, with its own audit
event, rather than widening `maint`.

### control (changes)

New endpoints under `/api/v1/maintenance`, admin role only. They accept
a Studio session (cookie + CSRF) or an enrolled owner token.

| Method and path | Body | Result |
|---|---|---|
| `GET /maintenance` | | `{open, expires_at, sessions}` |
| `POST /maintenance/window` | `{code}` | `201 {window_id, expires_at}` · `401 invalid_code` · `429 code_throttled` |
| `POST /maintenance/certificate` | `{window_id, public_key}` | `{certificate, valid_before, host_ca, host, port, user}` · `409 window_closed` |
| `DELETE /maintenance/window` | | `204` |

Control passes the account name as `actor` and the TLS peer address as
`client_addr`; it never sees the OTP secret or any CA key. Non-browser
clients send no `Origin` header, which the existing check already allows.

Also needed: the TLS server must send the device CA certificate in its
chain (or expose `GET /api/v1/pki/ca`, unauthenticated, CA only) so that
`stephctl login` can show its fingerprint before trusting it.

### Stephania Connect (changes)

- **Secret delivery.** `management.enroll` (BLE, sealed to the owner key)
  returns one more field, `otp_secret` (20 random bytes, base64). Connect
  stores it in the keychain with the device's token and CA pin. No QR
  code, no key shown to the user. A re-enroll rotates it.
- **Codes.** The device screen gets a "Maintenance code" row: the current
  6 digits, a countdown ring, tap to copy. Showing it requires Face ID or
  the passcode.
- **Clock.** The device may have no RTC battery and no internet, so its
  clock can be off. When Connect reaches the device over the LAN it
  records the offset from the HTTPS `Date` header and computes codes on
  the device's time. Without a recent offset it uses the phone's time and
  says so.
- **Fingerprint.** The device screen shows the CA fingerprint in the same
  short form as `stephctl login` (below).

## stephctl (CLI)

A single static binary for macOS, Linux and Windows. Rust, in this
workspace (`tools/stephctl`), built with rustls (pinned CA, no system
trust store), `ssh-key` for key generation and certificates, and mDNS
discovery. It wraps the system `ssh`/`scp` rather than implementing SSH.
Released with each `v*` tag next to the OS image.

### Commands

```text
stephctl login [<device>]          trust a device and sign in to it
stephctl logout [<device>]         forget the session (keeps the CA pin)
stephctl devices                   list known devices, session state
stephctl ssh [<device>] [-- <command…>]   open a maintenance shell or run a command
stephctl cp <src> <dst>            copy files (scp syntax; device:path)
stephctl window [<device>] [--close]      show or close the current window
stephctl forget <device>           remove the device, its pin and session
```

`<device>` is a name from `stephctl devices`, a `<host>.local` name or an
address. With one known device it can be left out; with none, `login`
browses `_stephania._tcp` and lists what it finds.

### `stephctl login`

1. Resolve the device (mDNS, or the given name/address, port 9443).
2. **First contact only:** fetch the CA certificate, compute its SHA-256
   and print it as 8 groups of 4 hex digits plus a short word form. The
   user compares it with the fingerprint on the device screen in Connect
   and confirms. The pin is saved; later connections fail hard if the CA
   changes (factory reset: `stephctl forget` and log in again).
3. Ask for username and password (`POST /api/v1/auth/session`). The
   account must be an admin to use `ssh`/`cp`.
4. Keep the session value and CSRF token in the OS credential store
   (macOS Keychain, Secret Service on Linux, Windows Credential Manager).
   The Studio's limits apply (12 h idle, 7 days absolute).

Files: `~/.config/stephctl/devices.toml` (name, addresses, port, CA pin;
no secrets), per-OS equivalent on Windows.

### `stephctl ssh`

1. Use the saved session; if it has expired, run the login prompt again.
2. If no window is open (`GET /maintenance`), prompt
   `Code from Stephania Connect:` and `POST /maintenance/window`.
3. Generate a fresh Ed25519 key pair in memory. Write the private key, the
   certificate and a `known_hosts` file into a new 0700 temporary
   directory.
4. `POST /maintenance/certificate` with the public key.
5. Run the system client:

   ```text
   ssh -F none -p <port> \
       -o IdentitiesOnly=yes -o IdentityAgent=none \
       -o IdentityFile=<tmp>/id -o CertificateFile=<tmp>/id-cert.pub \
       -o UserKnownHostsFile=<tmp>/known_hosts -o GlobalKnownHostsFile=none \
       -o StrictHostKeyChecking=yes -o HostKeyAlias=<host>.local \
       maint@<address> [-- command…]
   ```

   with `known_hosts` holding
   `@cert-authority <host>.local,<addresses> <host CA public key>`.
6. When `ssh` exits, overwrite and delete the temporary directory. The
   certificate expires within 5 minutes anyway.

`stephctl cp` does the same with `scp`. `--keep-window` leaves the window
open after exit; by default the CLI closes it when the last session it
started ends.

### Errors

| Situation | Message (exit code) |
|---|---|
| CA does not match the pin | `device certificate changed: refusing (run 'stephctl forget' after a factory reset)` (3) |
| Wrong code | `code not accepted` (4), up to 3 prompts |
| Throttled | `too many wrong codes, try again in <n> s` (4) |
| Not admin | `this account cannot open maintenance` (5) |
| Window closed meanwhile | asks for a new code (4) |
| `ssh` not found | `OpenSSH client not found in PATH` (6) |
| Device unreachable | `cannot reach <device> on the LAN` (2) |

## Revocation and reset

- Certificates expire in 5 minutes; closing a window stops sshd and ends
  its sessions.
- `stephania-maint` can add a certificate serial or key to the KRL
  (`revoked.krl`) for the rest of its validity.
- Factory reset deletes `/var/lib/stephania/maint`: new CAs, new OTP
  secret, new host key. Old pins in `stephctl` and Connect stop working.

## Threats considered

| Threat | Mitigation |
|---|---|
| Stolen Studio password | Also needs the code from the owner's phone |
| Stolen phone | Code display needs Face ID or the passcode; still needs the Studio password |
| Control plane compromised | Holds no CA key or OTP secret; can only relay a code the user types |
| Code phishing | LAN only, pinned TLS in `stephctl`; codes are single-use and live 30–90 s |
| Leaked certificate or key | Valid ≤ 5 min, bound to the client address, KRL |
| Port scanning from outside | sshd not running outside a window; nftables drops non-private sources |
| Host impersonation | Host certificate from the device's host CA; no TOFU prompt |

## Plan

1. `stephania-maint`: CAs, OTP, socket, audit; unit and polkit rule;
   nftables table; `stephania-sshd` unit and config; image changes.
2. Control endpoints and OpenAPI; `management.enroll` adds `otp_secret`
   (BLE contract and fixtures).
3. Connect: secret storage, code view with Face ID, clock offset,
   fingerprint.
4. `stephctl`: login, ssh, cp, window, devices; release builds.
5. CI: virtual-lab test opening a window, signing a certificate and
   logging in with the real OpenSSH client; negative tests (wrong code,
   replay, expired certificate, non-private source, window closed).

## Open questions

- Root access: a separate `maint-root` principal, or never?
- Should operators (not only admins) be able to read logs through a
  narrower principal?
- Window and certificate lifetimes (10 min / 5 min / 60 min) need a
  check against real maintenance work.
- Name of the CLI (`stephctl` is a working name).
