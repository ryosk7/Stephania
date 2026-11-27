// Tests the exact script embedded in dist/index.html.
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {webcrypto} from 'node:crypto';
import jsQR from 'jsqr';
import {describe, expect, it} from 'vitest';

const dist = new URL('../dist/', import.meta.url);
const html = readFileSync(new URL('index.html', dist), 'utf8');
const script = readFileSync(new URL('script.js', dist), 'utf8');
const ctx = {crypto: webcrypto, Uint8Array, Array};
vm.createContext(ctx);
vm.runInContext(script + '\n;globalThis.StephaniaSetup = StephaniaSetup;', ctx);
const S = ctx.StephaniaSetup;
const fixtures = JSON.parse(readFileSync(new URL('../../../contracts/fixtures/ble/setup-uris.json', import.meta.url), 'utf8'));

function decode(rows, scale = 4, quiet = 4) {
  const n = rows.length + 2 * quiet, size = n * scale, data = new Uint8ClampedArray(size * size * 4).fill(255);
  rows.forEach((row, r) => row.forEach((dark, c) => {
    if (!dark) return;
    for (let y = 0; y < scale; y++) for (let x = 0; x < scale; x++) {
      const i = (((r + quiet) * scale + y) * size + (c + quiet) * scale + x) * 4;
      data[i] = data[i + 1] = data[i + 2] = 0;
    }
  }));
  return jsQR(data, size, size)?.data;
}

describe('setup page', () => {
  it('embeds the exact tested script and pins it by CSP hash', async () => {
    expect(html).toContain(script);
    const {createHash} = await import('node:crypto');
    const hash = createHash('sha256').update(script).digest('base64');
    expect(html).toContain(`script-src 'sha256-${hash}'`);
    expect(html).toContain("default-src 'none'");
    // No external resources and no network APIs in the shipped page.
    const markup = html.replace(script, '');
    expect(markup).not.toMatch(/<(script|img|iframe)[^>]+src=|<link[^>]+href=|url\(/);
    expect(script).not.toMatch(/\bfetch\(|XMLHttpRequest|WebSocket|sendBeacon|EventSource|navigator\.share/);
  });

  it('produces URIs matching the shared fixtures format', async () => {
    for (const v of fixtures.valid.slice(0, 2)) {
      const secret = Uint8Array.from(v.secret.match(/../g).map(h => parseInt(h, 16)));
      expect(await S.uriFor(secret)).toBe(v.uri);
    }
  });

  it('generates distinct non-zero secrets', () => {
    const a = S.hex(S.newSecret()), b = S.hex(S.newSecret());
    expect(a).toMatch(/^[0-9a-f]{32}$/);
    expect(a).not.toBe(b);
  });

  it('QR round-trips through an independent decoder', async () => {
    for (let i = 0; i < 20; i++) {
      const uri = await S.uriFor(S.newSecret());
      expect(decode(S.matrix(uri))).toBe(uri);
    }
    const svg = S.svg(S.matrix('stephania-setup:v1:00112233445566778899aabbccddeeff:a8faed6a'));
    expect(svg).toMatch(/^<svg [^>]*viewBox="0 0 \d+ \d+"/);
  });
});
