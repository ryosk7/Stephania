# Licensing and supply chain

T3Box's own code is MIT (`LICENSE`). This page records what else ships,
under which terms, and what a release has to include. It is an engineering
review (R15), not legal advice; have it checked before a commercial
release.

## What is checked automatically

Workflow "Supply chain" (`.github/workflows/audit.yml`): on lockfile or
policy changes, nightly and on demand.

| Check | Tool | Fails on |
|---|---|---|
| Rust advisories, licences, bans, sources | `cargo deny --all-features check` (`deny.toml`) | any RustSec advisory, yanked crate, licence outside the allow-list, crates from other registries or git |
| Ruby advisories | `bundle-audit check --update` | any advisory for a locked gem |
| npm advisories | `npm audit --omit=dev --audit-level=high` per package | high/critical in shipped (production) dependencies |
| Inventory and SBOM | `ci/sbom.py` | a shipped Rust crate, npm package or gem without a licence in the policy |

`ci/sbom.py` writes `sbom.spdx.json` (SPDX 2.3), `THIRD_PARTY.md` and the
licence texts the components ship with (`LICENSES/`). Release artifacts use
the same script (§7: `sbom.spdx.json`, `LICENSES/`).

Allowed for Rust crates, npm packages and gems: MIT, Apache-2.0 (also WITH
LLVM-exception), BSD-2/3-Clause, ISC, Zlib, 0BSD, Unlicense, Unicode,
CDLA-Permissive-2.0 (Mozilla root store data in `webpki-roots`), CC0-1.0,
CC-BY-4.0 (`caniuse-lite` data, attribution in `THIRD_PARTY.md`),
BlueOak-1.0.0, Ruby, Python-2.0, MPL-2.0 (file-level copyleft; unmodified
use only). Anything else needs a review and a policy change in both
`deny.toml` and `ci/sbom.py`.

Current state (when this was written): no Rust advisories; the first run
of `bundle-audit` found ten advisories in Puma 6.6.1 (PROXY protocol
parser memory exhaustion, SSL connection memory exhaustion, among others),
fixed by moving to Puma 7.2.4 (config/puma.rb uses no hooks or option
reads affected by the 7.0 changes); npm
production trees have no high/critical findings. `apps/mobile` has three
moderate findings in Expo's build-time fingerprint tooling (`minimatch`
ReDoS); they do not ship in the app bundle and `npm audit` suggests no
compatible fix within SDK 58.

## Copyleft components and obligations

The engine binary dynamically links system libraries:

| Library | Licence (Debian build) | Used for |
|---|---|---|
| libx264 | GPL-2.0-or-later | H.264 encoding |
| libavcodec, libavutil (FFmpeg) | GPL-2.0-or-later as built by Debian (`--enable-gpl`); LGPL-2.1-or-later otherwise | AAC encoding, phone H.264 decoding |
| libasound | LGPL-2.1-or-later | ALSA capture |
| libv4l (via v4l crate, kernel interface) | LGPL-2.1-or-later | V4L2 capture |

Because of libx264 the shipped engine is a combined work under
GPL-2.0-or-later. MIT is GPL-compatible, so T3Box itself needs no change,
but every distribution of the OS image or the engine binary must:

1. include the licence texts (`LICENSES/`, plus `/usr/share/doc/*/copyright`
   inside the image);
2. provide the complete corresponding source: the T3Box source at the
   release tag, and the exact source packages of the GPL/LGPL system
   components (Debian and Raspberry Pi source packages at the versions listed in the image
   SBOM, `--dpkg` input of `ci/sbom.py`), either alongside the binaries or
   with a written offer valid for three years;
3. not add terms that restrict recipients' GPL rights (no EULA forbidding
   modification or reverse engineering of the image).

The OS image additionally contains the Linux kernel (GPL-2.0-only), BlueZ
(GPL-2.0-or-later), NetworkManager (GPL-2.0-or-later), systemd
(LGPL-2.1-or-later), RAUC (LGPL-2.1-only), glibc (LGPL-2.1-or-later), CRuby
(Ruby or BSD-2-Clause) and the Raspberry Pi firmware (proprietary,
redistributable under its own licence, Pi variant only). The same source
obligation applies to the GPL/LGPL parts.

Fonts: none ship. Text overlays are rendered by the browser on the user's
machine (docs/SCENE.md), so no font licence applies to the device.

T3Box Connect bundles two fonts so iPhone and Android look the same
(DESIGN.md §3.6): Inter and JetBrains Mono, both under the SIL Open Font
License 1.1 (via `@expo-google-fonts/*`, packaging MIT). OFL allows
embedding in an application; the fonts are not sold on their own and the
licence text ships in the app's third-party notices (`ci/sbom.py`).

H.264/AAC patents: encoding H.264 and AAC may require patent licences in
some jurisdictions depending on volume and use; this is independent of the
copyright licences above and must be reviewed for a commercial product.
