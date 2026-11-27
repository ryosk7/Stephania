// BlePlxTransport against the real Rust device core. FakeBle plays the role
// of the (future) BlueZ adapter exactly as contracts/ble-protocol.md
// specifies: reads pass through, the response to a completed write is
// notified on the same characteristic, fragmented under the request's id.
import {spawn, type ChildProcess} from 'node:child_process';
import {existsSync, mkdtempSync, readFileSync, rmSync} from 'node:fs';
import {connect, type Socket} from 'node:net';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {afterAll, beforeAll, describe, expect, it} from 'vitest';
import {fragment, fromHex, ProvisioningSession, toHex} from '@t3box/provisioning-client';
import {fromBase64, toBase64} from './base64';
import {BlePlxTransport, type BleLike} from './transport';

const bin = process.env.T3BOX_PROVISION_BIN;

class FakeBle implements BleLike {
  private buf = Buffer.alloc(0);
  private queue: ((f: Buffer) => void)[] = [];
  private n = 0;
  private monitors = new Map<string, (e: Error | null, c: {value: string | null} | null) => void>();
  writes = 0;
  constructor(private sock: Socket, private att: number) {
    sock.on('data', (d: Buffer) => { this.buf = Buffer.concat([this.buf, d]); this.drain(); });
  }
  private drain() {
    while (this.buf.length >= 4 && this.buf.length >= 4 + this.buf.readUInt32BE(0)) {
      const len = this.buf.readUInt32BE(0), f = this.buf.subarray(4, 4 + len);
      this.buf = this.buf.subarray(4 + len);
      this.queue.shift()?.(f);
    }
  }
  private call(method: string, params: object): Promise<any> {
    const body = Buffer.from(JSON.stringify({version: 1, request_id: 'b' + ++this.n, method, params}));
    const head = Buffer.alloc(4); head.writeUInt32BE(body.length);
    return new Promise(res => { this.queue.push(f => res(JSON.parse(f.toString()))); this.sock.write(Buffer.concat([head, body])); });
  }
  async readCharacteristicForDevice(_d: string, _s: string, c: string) {
    const r = await this.call('gatt.read', {uuid: c});
    if (!r.ok) throw new Error('ATT error');
    return {value: toBase64(fromHex(r.result.value_hex))};
  }
  async writeCharacteristicWithResponseForDevice(_d: string, _s: string, c: string, b64: string) {
    this.writes++;
    const frag = fromBase64(b64);
    if (frag.length > this.att) throw new Error('exceeds ATT payload');
    const r = await this.call('gatt.write', {uuid: c, fragment_hex: toHex(frag)});
    const response = r.ok ? (r.result.response_hex ? fromHex(r.result.response_hex) : null)
      : new TextEncoder().encode(JSON.stringify({error: r.error.code}));
    if (response) {
      const listener = this.monitors.get(c);
      // Notify asynchronously, like a radio would, after the write completes.
      setTimeout(() => fragment(frag[0], response, this.att).forEach(f => listener?.(null, {value: toBase64(f)})), 5);
    }
    return {};
  }
  monitorCharacteristicForDevice(_d: string, _s: string, c: string, l: (e: Error | null, c: {value: string | null} | null) => void) {
    this.monitors.set(c, l);
    return {remove: () => { if (this.monitors.get(c) === l) this.monitors.delete(c); }};
  }
}

describe('base64', () => {
  it('round-trips all lengths', () => {
    for (let n = 0; n < 40; n++) {
      const b = Uint8Array.from({length: n}, (_, i) => (i * 37 + n) & 255);
      expect(fromBase64(toBase64(b))).toEqual(b);
    }
    expect(toBase64(new TextEncoder().encode('t3box'))).toBe('dDNib3g=');
  });
});

describe.skipIf(!bin)('BlePlxTransport against Rust device core', () => {
  let dir: string, child: ChildProcess, sock: Socket;
  beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), 't3box-app-'));
    child = spawn(bin!, [], {env: {...process.env, T3BOX_PROVISION_SOCKET: join(dir, 'p.sock'), T3BOX_CREDENTIALS_DIR: dir,
      T3BOX_SETUP_FILE: join(dir, 'none'), T3BOX_NETWORK_BACKEND: 'virtual'}, stdio: 'ignore'});
    for (let i = 0; i < 100 && !existsSync(join(dir, 'p.sock')); i++) await new Promise(r => setTimeout(r, 30));
    sock = await new Promise<Socket>((ok, ko) => { const s = connect(join(dir, 'p.sock'), () => ok(s)); s.on('error', ko); });
  });
  afterAll(() => { sock?.destroy(); child?.kill(); rmSync(dir, {recursive: true, force: true}); });

  it('claims, scans and joins with notified responses at minimum MTU', async () => {
    const ble = new FakeBle(sock, 20);
    const session = new ProvisioningSession(new BlePlxTransport(ble, 'dev-1', 20));
    const secret = fromHex(readFileSync(join(dir, 'setup-secret'), 'utf8').trim());
    await expect(session.claim(new Uint8Array(16), 'app-phone')).rejects.toMatchObject({code: 'bad_proof'});
    await session.claim(secret, 'app-phone');
    expect(ble.writes).toBeGreaterThan(2);
    expect(await session.scan()).toEqual([{ssid: 'virtual-lab', rssi: -40, security: 'wpa-psk'}]);
    expect(await session.configureWifi({ssid: 'virtual-lab', security: 'wpa-psk', psk: 'password1'})).toBe('CONNECTED');
    expect((await session.deviceInformation()).network).toBe('CONNECTED');
  });
});
