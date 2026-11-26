# T3Box setup page

Single-file static page (`dist/index.html`) that generates the device setup secret **in the browser**, shows it as a QR code for the T3Box Connect app and downloads `t3box-setup.txt` for the microSD boot (FAT32) partition. Decided resolution of risk R02 without printed labels; see `contracts/ble-protocol.md` § Setup secret delivery.

- No network: CSP `default-src 'none'`, inline script pinned by its SHA-256; works offline and from `file://`.
- Format: `t3box-setup:v1:<32 hex>:<8 hex SHA-256 check>` (`contracts/fixtures/ble/setup-uris.json`).
- QR generation: vendored `qrcode-generator` 1.4.4 (MIT, Kazuhiko Arase), inlined at build.
- `npm test` builds the page and tests the exact embedded script: fixture format, randomness, and QR round-trip through an independent decoder (jsQR).

Publishing: `.github/workflows/setup-page-pages.yml` is manual-only and requires typing `publish`. A Pages site is public even for a private repository, so run it only after explicitly deciding to publish. Users can instead open the built `index.html` locally.
