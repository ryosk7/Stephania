// TypeScript phone client against the real Rust t3box-provision binary.
// Enabled when T3BOX_PROVISION_BIN points at a build with the ci-virtual feature.
import {spawn, type ChildProcess} from 'node:child_process';
import {mkdtempSync, readFileSync, rmSync, existsSync} from 'node:fs';
import {connect, type Socket} from 'node:net';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {afterAll, beforeAll, describe, expect, it} from 'vitest';
import {fromHex, ProvisioningError, ProvisioningSession, toHex, type BleTransport} from './index';

const bin = process.env.T3BOX_PROVISION_BIN;

class SocketTransport implements BleTransport {
  readonly attPayload = 20; // worst case: minimum MTU
  private buf = Buffer.alloc(0);
  private waiting?: (b: Buffer) => void;
  private n = 0;
  constructor(private readonly sock: Socket) {
    sock.on('data', d => { this.buf = Buffer.concat([this.buf, d]); this.drain(); });
  }
  private drain() {
    if (this.buf.length < 4) return;
    const len = this.buf.readUInt32BE(0);
    if (this.buf.length < 4 + len) return;
    const frame = this.buf.subarray(4, 4 + len);
    this.buf = this.buf.subarray(4 + len);
    const w = this.waiting; this.waiting = undefined; w?.(frame);
  }
  private call(method: string, params: object): Promise<Record<string, any>> {
    const body = Buffer.from(JSON.stringify({version: 1, request_id: 'ts' + ++this.n, method, params}));
    const head = Buffer.alloc(4); head.writeUInt32BE(body.length);
    return new Promise(resolve => { this.waiting = f => resolve(JSON.parse(f.toString())); this.sock.write(Buffer.concat([head, body])); });
  }
  async read(uuid: string) {
    const r = await this.call('gatt.read', {uuid});
    if (!r.ok) throw new ProvisioningError(r.error.code);
    return fromHex(r.result.value_hex);
  }
  async write(uuid: string, fragment: Uint8Array) {
    const r = await this.call('gatt.write', {uuid, fragment_hex: toHex(fragment)});
    if (!r.ok) throw new ProvisioningError(r.error.code);
    return r.result.response_hex ? fromHex(r.result.response_hex) : null;
  }
}

describe.skipIf(!bin)('interop with Rust t3box-provision', () => {
  let dir: string, child: ChildProcess, sock: Socket;
  beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), 't3box-ts-'));
    child = spawn(bin!, [], {env: {...process.env, T3BOX_PROVISION_SOCKET: join(dir, 'p.sock'), T3BOX_CREDENTIALS_DIR: dir, T3BOX_NETWORK_BACKEND: 'virtual'}, stdio: 'ignore'});
    for (let i = 0; i < 100 && !existsSync(join(dir, 'p.sock')); i++) await new Promise(r => setTimeout(r, 30));
    sock = await new Promise<Socket>((resolve, reject) => { const s = connect(join(dir, 'p.sock'), () => resolve(s)); s.on('error', reject); });
  });
  afterAll(() => { sock?.destroy(); child?.kill(); rmSync(dir, {recursive: true, force: true}); });

  it('claims, scans and joins at minimum MTU', async () => {
    const session = new ProvisioningSession(new SocketTransport(sock));
    expect((await session.deviceInformation()).ownership).toBe('UNCLAIMED');
    const secret = fromHex(readFileSync(join(dir, 'setup-secret'), 'utf8').trim());
    await expect(session.claim(new Uint8Array(16), 'ts-phone')).rejects.toMatchObject({code: 'bad_proof'});
    const key = await session.claim(secret, 'ts-phone');
    expect(key.length).toBe(32);
    expect((await session.deviceInformation()).ownership).toBe('CLAIMED');
    await expect(session.claim(secret, 'intruder')).rejects.toMatchObject({code: 'already_claimed'});
    expect(await session.scan()).toEqual([{ssid: 'virtual-lab', rssi: -40, security: 'wpa-psk'}]);
    expect(await session.configureWifi({ssid: 'elsewhere', security: 'wpa-psk', psk: 'password1'})).toBe('FAILED');
    expect((await session.networkStatus()).error).toBe('join_failed');
    expect(await session.configureWifi({ssid: 'virtual-lab', security: 'wpa-psk', psk: 'password1'})).toBe('CONNECTED');
    // A session with the wrong owner key is rejected by the device.
    const other = new ProvisioningSession(new SocketTransport(sock));
    other.useOwnerKey(new Uint8Array(32));
    await expect(other.scan()).rejects.toMatchObject({code: 'bad_proof'});
  });
});
