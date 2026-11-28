# Studio accounts, sessions and roles

The control plane (services/control) accepts three kinds of credentials.

| Credential | Who | How it is presented | Scope |
|---|---|---|---|
| Bootstrap token (`STEPHANIA_CONTROL_TOKEN`) | Device owner, local maintenance | `Authorization: Bearer`, loopback listener only | admin |
| Enrolled token (`t3b_…`, BLE `management.enroll`) | Owner's phone / LAN clients | `Authorization: Bearer`, TLS only | admin |
| Studio account session | People using the Studio | `stephania_session` cookie + `X-CSRF-Token` | its role |
| Camera credential (`t3c_…`) | The phone camera | `Authorization: Bearer` | camera session only |

## Loopback and LAN

Whether a request is local is decided from the connection, never from
headers: it is local only when it arrived on the plain-HTTP listener (bound
to 127.0.0.1) from a loopback TCP peer. Everything else, including any
request on the TLS listener (port 9443) whatever its `Host`, is LAN and
must be TLS to a certified device name or address. Local requests must
name `localhost` or `127.0.0.1` in `Host` (DNS-rebinding defence). There is
no reverse proxy, so `Forwarded` and `X-Forwarded-*` (host, proto, for) are
ignored everywhere: they change neither the host, the scheme nor the client
address.

## Accounts

- Created by an admin (`POST /api/v1/users`), or with the owner token for
  the first account. Username `^[a-z0-9][a-z0-9_.-]{0,31}$`, password
  10–256 characters.
- Passwords are stored as scrypt (N=2^14, r=8, p=1, 16-byte random salt),
  encoded as `scrypt$N$r$p$salt$hash` so the cost can be raised later.
- At least one enabled admin account always remains (`409 last_admin`).
- Changing a user's role, disabling them or resetting their password ends
  all of their sessions and open event streams immediately. Changing your
  own password (`PUT /api/v1/auth/password`) ends your other sessions.

## Roles

| | admin | operator |
|---|---|---|
| Read status, scenes, audio, events | ✓ | ✓ |
| Start/stop stream, Program transition, audio gain/mute/inputs | ✓ | ✓ |
| Create/edit/delete/import scenes | ✓ | – |
| Users, camera credentials | ✓ | – |

Operators are for running an event without being able to change its
layouts. The rule is enforced in the API (`403 forbidden_role`); the Studio
hides what an operator cannot do. Role checks are made against the route
the request is dispatched to (e.g. `/api/v1/users`), not the raw path, so
percent-encoded spellings such as `/api/v1/%75sers` get the same answer.

## Sessions

- `POST /api/v1/auth/session {username, password}` sets `stephania_session`:
  256-bit random value, only its SHA-256 is stored; `HttpOnly`,
  `SameSite=Strict`, `Path=/api/`, `Secure` whenever the request is TLS.
  On the LAN the API is TLS-only, so the cookie is always `Secure` there.
- 12 h idle timeout, 7 day absolute lifetime, at most 8 sessions per user
  (oldest dropped).
- Every state-changing request authenticated by the cookie must carry the
  session's CSRF token in `X-CSRF-Token` (returned at sign-in and by
  `GET /api/v1/auth/me`). This is in addition to the strict same-origin
  check every request already gets and to `SameSite=Strict`.
- `DELETE /api/v1/auth/session` signs out and closes that session's event
  stream.

## Login throttling

Five failures for a username, or from one client address, lock that key
for 60 s, doubling to at most 15 min (`429 login_throttled`, `Retry-After`).
The client address is the TCP peer, never `X-Forwarded-For`. Only
well-formed usernames get a username key. Unknown usernames cost the same
scrypt work as known ones and get the same `401 invalid_login`, so the
response does not reveal which usernames exist.

The counters are in memory (reset on restart); at most 4096 keys of at most
128 bytes are kept. A full table evicts the least recently failed key that
is not locked; a running lock is never evicted, and when every key is
locked, new keys are refused until the first lock expires.

Password checks are bounded overall, because scrypt holds Ruby's global
lock and would stall every other request: at most 2 run at once and 2 per
second on average (bursts of 10). Beyond that a sign-in is refused without
any work (`429 login_busy`, `Retry-After: 1`) and does not count as a
failure.

## Event streams

`GET /api/v1/events` allows 8 streams in total. One credential (a session
or a token) holds at most 3 and one account at most 4; a new stream beyond
that replaces that credential's or account's oldest stream, so a client
that reconnects also frees the slot its dead connection still held.
Operators cannot take the last 2 slots, which stay for admins. Idle streams
send a keepalive every 15 s, and TCP_USER_TIMEOUT (45 s) drops peers that
stopped acknowledging. Every 15 s the stream re-checks its credential and
ends once the session has expired (idle or absolute) or the enrolled token
was revoked; sign-out, password changes and account changes end streams
immediately.

## Request bodies

Puma refuses bodies over 8 MiB (the image upload limit) before buffering
them, and chunked bodies as soon as they pass it. Before anything is
parsed, every route is limited to 64 KiB except `POST /api/v1/assets`
(8 MiB) (`413 body_too_large`), and `multipart/form-data` is refused
everywhere (`415 unsupported_media_type`).

## Studio

The Studio signs in with an account; a "Connect with the admin token" fallback is
kept for first-time setup and maintenance. Routes: `#/` dashboard,
`#/studio` scenes and layout, `#/audio` program audio, `#/settings`
password and (admins) users.

## Phone app and PCs

The phone app (apps/mobile) talks to the management API only through its
native pinned client (`modules/stephania-pinned-fetch`): the TLS chain must end
in the device CA whose SHA-256 the device delivered sealed over BLE
(`management.enroll`), the leaf must validate with that CA as the only
anchor (dates, signatures, host name), and system roots, proxies, cookies
and redirects are not used. It tries the device's URLs in order (name, then
addresses) and never falls back to weaker trust.

A PC has no such pin. `GET /ca.pem` and `GET /ca.crt` (no credentials; on
the LAN only over TLS to a certified name) serve the device CA certificate
in PEM and DER. The download is not authenticated: the PC does not trust
the device yet (the browser shows a certificate warning), and anyone on the
LAN can answer mDNS for `<host>.local` and serve their own CA. The only
protection is comparing fingerprints, so the phone app's "Open on PC"
screen shows the URL and the full CA SHA-256 and walks the user through it:

1. Join the same network and download `https://<host>.local:9443/ca.crt`.
   Accept the browser warning for this download only; do not sign in there.
2. Hash the file: `certutil -hashfile stephania-ca.crt SHA256` (Windows),
   `shasum -a 256 stephania-ca.crt` (macOS), `sha256sum stephania-ca.crt`
   (Linux). The DER file's hash is the certificate fingerprint.
3. Install it as a trusted root only if all 64 hex characters match the
   phone; otherwise delete it, since something on the network is
   impersonating the device.

What a PC trusts by installing the device CA (crate::pki):

- Extended key usage `serverAuth` only, path length 0, key usage
  `keyCertSign`/`cRLSign`: on platforms that enforce EKU chaining (Windows,
  macOS, Chrome, OpenSSL) it cannot vouch for code signing, S/MIME or
  client authentication.
- Name constraints: DNS `<host>.local`, and the IPv4 private and
  link-local ranges and IPv6 ULA (10/8, 172.16/12, 192.168/16, 169.254/16,
  fd00::/8), because the phone and `stephctl` reach the device by address
  when mDNS does not resolve, and the pin must survive DHCP changes. A
  permitted e-mail subtree `invalid` excludes every real address. URI and
  directory-name forms are not constrained (rcgen cannot encode URI
  constraints, and rustls rejects every directory-name constraint), so they
  rely on the EKU.
- 10-year validity; the CA key never leaves the device (`ca-key.pem`,
  0600 root).

Residual risk: anyone who obtains `ca-key.pem` (e.g. by reading the SD
card) can issue certificates for other hosts' private addresses, such as a
router at `192.168.1.1`, that PCs which installed the CA accept. Remove the
certificate from a PC when it no longer manages the device. The CA is not
replaced on a factory reset.

The CA changes only when the device's host name changes (its name
constraint cannot cover the new name; logged, and phones and PCs must be
set up again) or when no client can have pinned it yet. A missing or
damaged CA file is otherwise an error, not a new CA: `ca.pem` is restored
from the chain in `server.pem` when possible, and a lost `ca-key.pem` stops
`stephania-pki` until it is restored or the PKI directory is removed.
