# Stephania setup page

Single-file static page (`dist/index.html`) that generates the device setup secret **in the browser**, shows it as a QR code for the Stephania Connect app and downloads `stephania-setup.txt` for the microSD boot (FAT32) partition. Decided resolution of risk R02 without printed labels; see `contracts/ble-protocol.md` § Setup secret delivery.

- No network: CSP `default-src 'none'`, inline script pinned by its SHA-256; works offline and from `file://`.
- Format: `stephania-setup:v1:<32 hex>:<8 hex SHA-256 check>` (`contracts/fixtures/ble/setup-uris.json`).
- QR generation: vendored `qrcode-generator` 1.4.4 (MIT, Kazuhiko Arase), inlined at build.
- `npm test` builds the page and tests the exact embedded script: fixture format, randomness, and QR round-trip through an independent decoder (jsQR).

- Design: the tokens and components of DESIGN.md §4–5 (same values as Connect), light and dark following the system with a three-way switch stored in the browser, Lucide icons inlined. No fonts or images are loaded, so the system UI font is used.

Publishing: `.github/workflows/setup-page-pages.yml` deploys every change on main (or a manual run) from `dist/` to https://ryosk7.github.io/Stephania/. Users can instead open the built `index.html` locally.
