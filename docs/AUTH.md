# Studio accounts, sessions and roles

The control plane (services/control) accepts three kinds of credentials.

| Credential | Who | How it is presented | Scope |
|---|---|---|---|
| Bootstrap token (`T3BOX_CONTROL_TOKEN`) | Device owner, local maintenance | `Authorization: Bearer`, loopback only | admin |
| Enrolled token (`t3b_…`, BLE `management.enroll`) | Owner's phone / LAN clients | `Authorization: Bearer`, TLS only | admin |
| Studio account session | People using the Studio | `t3box_session` cookie + `X-CSRF-Token` | its role |
| Camera credential (`t3c_…`) | The phone camera | `Authorization: Bearer` | camera session only |

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
hides what an operator cannot do.

## Sessions

- `POST /api/v1/auth/session {username, password}` sets `t3box_session`:
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
Unknown usernames cost the same scrypt work as known ones and get the same
`401 invalid_login`, so the response does not reveal which usernames exist.
The counters are in memory (reset on restart); at most 4096 keys are kept.

## Studio

The Studio signs in with an account; a "管理トークンで接続" fallback is
kept for first-time setup and maintenance. Routes: `#/` dashboard,
`#/studio` scenes and layout, `#/audio` program audio, `#/settings`
password and (admins) users.

## Phone app and PCs

The phone app (apps/mobile) talks to the management API only through its
native pinned client (`modules/t3box-pinned-fetch`): the TLS chain must end
in the device CA whose SHA-256 the device delivered sealed over BLE
(`management.enroll`), the leaf must validate with that CA as the only
anchor (dates, signatures, host name), and system roots, proxies, cookies
and redirects are not used. It tries the device's URLs in order (name, then
addresses) and never falls back to weaker trust.

A PC has no such pin. `GET /ca.pem` (no credentials; on the LAN only over
TLS to a certified name) serves the device CA certificate; the phone app's
"PCのブラウザで開く" screen shows the URL and the CA fingerprint so the user
can compare before installing it as a trusted root.
