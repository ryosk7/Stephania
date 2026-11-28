# Maintenance shell access (design)

Status: steps 1 and 2 of the [plan](#plan) are implemented.
`stephania-maint` (`src/maint.rs`, `src/sshcert.rs`, `src/totp.rs`),
`stephania-sshd`, the polkit rule and the nftables table are in the image,
tested by `ci/maint-ssh-e2e.sh` (real OpenSSH) and the QEMU smoke test.
Control serves `/api/v1/maintenance` (tested against the real
`stephania-maint`), and `management.enroll` delivers the code secret to
Connect (tested from the TypeScript client through the real
`stephania-provision` and `stephania-maint`). Connect does not show codes
yet, and `stephctl` and `steph` are not built.

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
- Root by default: root is a separate, explicit request (see
  [Privileges](#privileges)).
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
  `stephania-maint` user manage that unit, restart the units in the
  [steph allowlist](#what-steph-may-change), install RAUC bundles and
  reboot, and nothing else.
- A second socket, `/run/stephania-maint/ops.sock`, group `maint` (0660),
  is the broker for `steph` (see [steph](#steph-on-device-cli)). The
  service is in group `stephania` to read engine status.

Socket operations:

| Operation | Request | Result |
|---|---|---|
| `window.open` | `{actor, client_addr, code, root}` | `{window_id, expires_at, root}`; starts sshd |
| `cert.sign` | `{window_id, public_key, client_addr, root}` | `{certificate, valid_before, host_ca, principal, port}` |
| `window.close` | `{window_id?, actor}` | `{closed}`; stops sshd and ends its sessions |
| `status` | `{}` | `{open, window, code_locked_until}` |
| `otp.rotate` | `{}` | `{otp_secret}` (hex); closes any open window |

Peers are told apart by uid: the control plane (`STEPHANIA_IPC_ALLOWED_UIDS`)
may call the first four, the provisioning core
(`STEPHANIA_MAINT_ENROLL_UIDS`, `stephania-provision` in group
`stephania-maint`) only `otp.rotate`, which `management.enroll` calls to
give each new Connect enrollment a fresh secret.

Rules enforced here, not in control:

- **Code check.** TOTP (RFC 6238, SHA-1, 6 digits, 30 s step), accepting
  the current step and one step either side. A step that has been used
  once is rejected afterwards (replay). Five wrong codes lock code entry
  for 60 s, doubling to 15 min, the same curve as Studio sign-in.
- **Window.** One window at a time, 10 minutes to start sessions, closed
  early by `window.close`. Sessions started in the window may run to an
  absolute limit of 60 minutes; closing the window or reaching the limit
  ends them (`KillMode=control-group`).
- **Certificate.** Signed with the user CA: principal `maint`, or
  `maint-root` when the window was opened with `root` (see
  [Privileges](#privileges)), validity
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
AuthorizedPrincipalsFile /etc/stephania/ssh_principals/%u   # maint: "maint"; root: "maint-root"
AuthorizedKeysFile none
AuthenticationMethods publickey
PubkeyAcceptedAlgorithms ssh-ed25519-cert-v01@openssh.com
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password   # certificate only, principal maint-root
AllowUsers maint root
DisableForwarding yes
PermitUserRC no
MaxAuthTries 2
LoginGraceTime 20
ClientAliveInterval 30
ClientAliveCountMax 4
ExposeAuthInfo yes                  # steph reads the certificate key ID for audit
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

Two levels, chosen when the window is opened. A window is one or the
other; switching means closing it and opening a new one with a new code.

**`maint` (default).** Not root, no `sudo`.

- Member of `systemd-journal` (read all logs).
- Changes go through `steph`, which asks `stephania-maint`; the `maint`
  user itself has no polkit rights.
- Read access to `/var/lib/stephania` except key material.

**`root` (explicit).** Logs in as `root` with a certificate for the
principal `maint-root`; `/etc/stephania/ssh_principals/root` lists only
that principal, so a `maint` certificate can never log in as root.

- Requested with `stephctl ssh --root`; the window request carries
  `root: true` and Connect is not involved beyond the normal code.
- Admin role required, as for `maint`. The window lasts 5 minutes to start
  sessions instead of 10, and sessions end after 30 minutes instead of 60.
- Audited as its own event type (`maintenance.root_window_opened`,
  `maintenance.root_certificate_issued`), shown in the Studio event log
  with the account name.
- Root can change the running system (the root file system is read-only
  SquashFS with an overlay, so changes outside `/var/lib/stephania` are
  lost on reboot or update). It is the escape hatch, not the way to do
  routine work.

### control (changes)

New endpoints under `/api/v1/maintenance`, admin role only. They accept
a Studio session (cookie + CSRF) or an enrolled owner token.

| Method and path | Body | Result |
|---|---|---|
| `GET /maintenance` | | `{open, window, code_locked_until}` |
| `POST /maintenance/window` | `{code, root?}` | `201 {window_id, expires_at, sessions_end_at, root, user}` · `401 invalid_code` · `409 window_open` · `429 code_throttled` (`Retry-After`) |
| `POST /maintenance/certificate` | `{window_id, public_key}` | `{certificate, valid_before, serial, host_ca, principal, user, host, port}` (`user` is `maint` or `root`) · `409 window_closed` |
| `DELETE /maintenance/window` | | `204` |

`503 maintenance_unavailable` when `stephania-maint` does not answer,
`503 maintenance_failed` when sshd could not be started. Opening and
closing are also published on `/api/v1/events`.

Control passes the account name (`owner` for an owner token) as `actor`
and the TCP peer address (never `X-Forwarded-For`) as `client_addr`; it never sees the OTP secret or any CA key. Non-browser
clients send no `Origin` header, which the existing check already allows.

`stephctl login` gets the device CA from the existing unauthenticated
`GET /ca.pem` (the TLS server also sends it in its chain) and shows its
fingerprint before trusting it.

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
stephctl ssh [<device>] [--root] [-- <command…>]   open a maintenance shell or run a command
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
2. If no window of the requested level is open (`GET /maintenance`),
   prompt `Code from Stephania Connect:` and `POST /maintenance/window`
   (with `root: true` for `--root`; the prompt then says `root window`).
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
       maint@<address> [-- command…]          # root@<address> with --root
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

## steph (on-device CLI)

`steph` is the command people use once they are on the device. It covers
routine maintenance so that a root window is rarely needed, and it is the
same for `maint` and `root` sessions. One Rust binary,
`/usr/bin/steph` (`src/bin/steph.rs`), reusing the engine crate's IPC,
`bootctl` and NetworkManager code.

`stephctl` runs on the laptop and gets you in; `steph` runs on the device
once you are in. `stephctl ssh -- steph status` runs one command.

### Principles

- **Reads directly, changes through the broker.** Read-only commands read
  the journal, `/proc`, `rauc status`, `stephania-bootctl` state,
  NetworkManager and `vcgencmd` themselves. Anything that changes the
  device goes to `stephania-maint` over `ops.sock`, which checks it,
  performs it and audits it. Root uses the same path, so its routine
  changes are audited the same way.
- **Audited with the certificate.** sshd's `ExposeAuthInfo` gives the
  session's certificate; `steph` sends its key ID (`<account>@<window>`)
  with every change, and `stephania-maint` logs it with the result.
- **Broadcast first.** A command that would interrupt the stream (restart
  the engine or the phone receiver, reboot, apply an update) is refused
  while on air. `--while-live` overrides it after the device name is
  typed back.
- **Machine-readable.** Every read command takes `--json`; the shapes are
  in `contracts/steph.schema.json` and are the same the maintenance API
  would return, so a Studio page can reuse them later.
- Plain output, no colour when not a TTY, exit code 0 on success.

### Commands

```text
steph status                    one screen: version, slot, uptime, engine and
                                stream state, sources, network, temperature,
                                throttling, data partition use, window time left
steph whoami                    session: user, level (maint/root), account,
                                window ID, certificate expiry, window time left
steph version                   OS version, slot, engine/control/Studio versions

steph logs [<service>] [-f] [-n <lines>] [--since <time>] [--boot <n>]
                                journal of one service or of all Stephania
                                units; services by short name (below)
steph services                  every Stephania unit: active state, since,
                                restarts, memory
steph restart <service>         restart one service (allowlist, broker)
steph reboot                    reboot into the committed slot (broker)

steph update status             slots A/B, booted, primary, state, trial boot
steph update apply <bundle>     verify and install a .raucb, then trial boot
                                (copy it first with `stephctl cp`)

steph net                       interfaces, addresses, Wi-Fi SSID and signal,
                                mDNS name, management URLs, default route
steph hw                        temperature, throttling history, bootloader,
                                USB tree, capture devices (V4L2), audio inputs
steph time                      device clock, NTP sync, RTC presence and
                                battery, offset to the laptop (given by stephctl)
steph disk                      data partition and overlay usage

steph diag [-o <file>|-]        evidence bundle (tar.gz) as collect.sh does,
                                secrets excluded; `-` writes to stdout:
                                stephctl ssh -- steph diag -o - > diag.tar.gz

steph window close              end this window and its sessions now
```

Service short names: `engine` (`stephania`), `control`, `phone`, `ble`,
`provision`, `pki`, `update` (`stephania-update-confirm` and the deadline
timer).

### What steph may change

| Command | Done by | `maint` | `root` | Refused while on air |
|---|---|---|---|---|
| `restart control`, `ble`, `provision`, `pki` | broker | yes | yes | no |
| `restart engine`, `phone` | broker | yes | yes | yes |
| `reboot` | broker | yes | yes | yes |
| `update apply` | broker (`stephania-update`) | yes | yes | yes |
| `window close` | broker | yes | yes | no |
| everything else that changes the system | root shell | no | yes | — |

Not in `steph` on purpose: Wi-Fi changes, accounts, tokens and factory
reset. They stay with the owner-authenticated paths (Connect over BLE,
the Studio, the physical reset button), so a maintenance window cannot
take over the device.

`ops.sock` operations: `ops.restart {unit, actor, force}`,
`ops.reboot {actor, force}`, `ops.update_apply {path, actor, force}`,
`ops.window_close {actor}`. `unit` must be in the allowlist; `path` must be
a regular file under `/var/lib/stephania/inbox` and is verified by RAUC
as usual. The inbox (group `maint`, 0770, cleared at boot and when the
window closes) is where `stephctl cp <file> device:` puts uploads by
default; it is separate from `/var/lib/stephania/maint`, which only the
broker can read.

### Errors

| Situation | Message (exit code) |
|---|---|
| Change refused while on air | `on air: this would interrupt the stream (use --while-live)` (3) |
| Unit not in the allowlist | `steph cannot restart <unit>` (4) |
| Bundle rejected by RAUC | RAUC's message (5) |
| Broker unreachable | `stephania-maint is not running` (6) |
| Window expired during the command | `maintenance window closed` (7) |

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
| `maint` certificate used for root | Root accepts only the `maint-root` principal, issued only for a root window |

## Plan

1. `stephania-maint`: CAs, OTP, socket, audit; unit and polkit rule;
   nftables table; `stephania-sshd` unit and config; image changes.
2. Control endpoints and OpenAPI; `management.enroll` adds `otp_secret`
   (BLE contract and fixtures).
3. Connect: secret storage, code view with Face ID, clock offset,
   fingerprint.
4. `stephctl`: login, ssh, cp, window, devices; release builds.
   `steph`: read commands first (status, logs, services, update status,
   net, hw, time, disk, diag), then the brokered changes.
5. CI: virtual-lab test opening a window, signing a certificate and
   logging in with the real OpenSSH client; negative tests (wrong code,
   replay, expired certificate, non-private source, window closed).

## Open questions

- Should operators (not only admins) be able to read logs through a
  narrower principal?
- Window and certificate lifetimes (10 min / 5 min / 60 min) need a
  check against real maintenance work.
- Name of the CLI (`stephctl` is a working name).
