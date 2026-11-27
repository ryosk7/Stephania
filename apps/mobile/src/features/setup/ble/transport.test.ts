// BlePlxTransport (protocol v3) against the real Rust device core. FakeBle
// plays the future BlueZ adapter exactly as contracts/ble-protocol.md
// specifies: written fragments are forwarded; the response is indicated on
// the RPC characteristic, fragmented under the request's message id. It can
// also lose or duplicate indications to exercise recovery.
import {spawn, type ChildProcess} from 'node:child_process';
import {existsSync, mkdtempSync, rmSync} from 'node:fs';
import {randomBytes} from 'node:crypto';
import {connect, type Socket} from 'node:net';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {afterAll, beforeAll, describe, expect, it} from 'vitest';
import {fragment, fromHex, ProvisioningSession, toHex, UUID} from '@stephania/provisioning-client';
import {fromBase64, toBase64} from './base64';
import {BlePlxTransport, type BleLike} from './transport';

const bin = process.env.STEPHANIA_PROVISION_BIN;

class FakeBle implements BleLike {
  private buf = Buffer.alloc(0);
  private queue: ((f: Buffer) => void)[] = [];
  private n = 0;
  private listener?: (e: Error | null, c: {value: string | null} | null) => void;
  peer = 'AA:00:00:00:00:01';
  loseNextResponse = false;
  duplicateIndications = false;
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
  async writeCharacteristicWithResponseForDevice(_d: string, service: string, c: string, b64: string) {
    if (service !== UUID.service || c !== UUID.rpc) throw new Error('unknown characteristic');
    this.writes++;
    const frag = fromBase64(b64);
    if (frag.length > this.att) throw new Error('exceeds ATT payload');
    const r = await this.call('gatt.request', {peer: this.peer, fragment_hex: toHex(frag)});
    if (!r.ok) throw new Error('ATT error');
    if (r.result.response_hex) {
      if (this.loseNextResponse) { this.loseNextResponse = false; return {}; }
      const frags = fragment(frag[0], fromHex(r.result.response_hex), this.att);
      setTimeout(() => {
        for (const f of frags) {
          this.listener?.(null, {value: toBase64(f)});
          if (this.duplicateIndications) this.listener?.(null, {value: toBase64(f)});
        }
      }, 2);
    }
    return {};
  }
  monitorCharacteristicForDevice(_d: string, _s: string, _c: string, l: (e: Error | null, c: {value: string | null} | null) => void) {
    this.listener = l;
    return {remove: () => { if (this.listener === l) this.listener = undefined; }};
  }
  async reconnectLink() {
    await this.call('gatt.disconnect', {peer: this.peer});
    this.peer = this.peer.replace(/..$/, m => String(Number(m) + 1).padStart(2, '0'));
  }
}

describe('base64', () => {
  it('round-trips all lengths', () => {
    for (let n = 0; n < 40; n++) {
      const b = Uint8Array.from({length: n}, (_, i) => (i * 37 + n) & 255);
      expect(fromBase64(toBase64(b))).toEqual(b);
    }
    expect(toBase64(new TextEncoder().encode('stephania'))).toBe('c3RlcGhhbmlh');
  });
});

describe.skipIf(!bin)('BlePlxTransport against Rust device core', () => {
  let dir: string, child: ChildProcess, sock: Socket;
  beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), 'stephania-app-'));
    child = spawn(bin!, [], {env: {...process.env, STEPHANIA_PROVISION_SOCKET: join(dir, 'p.sock'), STEPHANIA_CREDENTIALS_DIR: dir, STEPHANIA_NETWORK_BACKEND: 'virtual'}, stdio: 'ignore'});
    for (let i = 0; i < 100 && !existsSync(join(dir, 'p.sock')); i++) await new Promise(r => setTimeout(r, 30));
    sock = await new Promise<Socket>((ok, ko) => { const s = connect(join(dir, 'p.sock'), () => ok(s)); s.on('error', ko); });
  });
  afterAll(() => { sock?.destroy(); child?.kill(); rmSync(dir, {recursive: true, force: true}); });

  it('claims, scans and joins at minimum MTU, surviving lost and duplicated indications', async () => {
    const ble = new FakeBle(sock, 20);
    const transport = new BlePlxTransport(ble, 'dev-1', 20, () => ble.reconnectLink());
    const session = new ProvisioningSession(transport);
    ble.duplicateIndications = true;
    await session.claim('app-phone', randomBytes(32));
    await expect(session.claim('app-phone-2', randomBytes(32))).rejects.toMatchObject({code: 'already_claimed'});
    ble.duplicateIndications = false;
    expect(ble.writes).toBeGreaterThan(2);
    expect(await session.scan()).toEqual([{ssid: 'virtual-lab', rssi: -40, security: 'wpa-psk'}]);
    // The join succeeds on the device but its indication is lost: the
    // transport times out, reconnects (new address) and the identical retry
    // is replayed by the device.
    const shortTimeout = new ProvisioningSession({
      request: (id, m) => transport.request(id, m, 300),
      reconnect: () => transport.reconnect(),
    });
    shortTimeout.useOwnerKey((session as unknown as {ownerKey: Uint8Array}).ownerKey);
    ble.loseNextResponse = true;
    expect(await shortTimeout.configureWifi({ssid: 'virtual-lab', security: 'wpa-psk', psk: 'password1'})).toBe('CONNECTED');
    expect(ble.peer).toBe('AA:00:00:00:00:02');
    expect((await session.deviceInformation()).network).toBe('CONNECTED');
    transport.close();
  });
});
