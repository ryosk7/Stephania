// TypeScript phone client (protocol v2) against the real Rust t3box-provision
// binary. Enabled when T3BOX_PROVISION_BIN points at a ci-virtual build.
import {spawn, type ChildProcess} from 'node:child_process';
import {existsSync, mkdtempSync, readFileSync, rmSync} from 'node:fs';
import {connect, type Socket} from 'node:net';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {afterAll, beforeAll, describe, expect, it} from 'vitest';
import {fragment, fromHex, ProvisioningError, ProvisioningSession, toHex, type BleTransport} from './index';

const bin = process.env.T3BOX_PROVISION_BIN;

/** Plays the BLE adapter: forwards 20-byte fragments over the device socket. */
class SocketTransport implements BleTransport {
  private buf = Buffer.alloc(0);
  private waiting: ((b: Buffer) => void)[] = [];
  private n = 0;
  /** Lose the response of the k-th upcoming request (0 = next). */
  private dropAt = -1;
  loseResponseOf(k: number) { this.dropAt = k; }
  requests: number[] = [];
  reconnects = 0;
  constructor(private readonly sock: Socket, private peer: string) {
    sock.on('data', (d: Buffer) => { this.buf = Buffer.concat([this.buf, d]); this.drain(); });
  }
  private drain() {
    while (this.buf.length >= 4 && this.buf.length >= 4 + this.buf.readUInt32BE(0)) {
      const len = this.buf.readUInt32BE(0), f = this.buf.subarray(4, 4 + len);
      this.buf = this.buf.subarray(4 + len);
      this.waiting.shift()?.(f);
    }
  }
  private call(method: string, params: object): Promise<Record<string, any>> {
    const body = Buffer.from(JSON.stringify({version: 1, request_id: 'ts' + ++this.n, method, params}));
    const head = Buffer.alloc(4); head.writeUInt32BE(body.length);
    return new Promise(resolve => { this.waiting.push(f => resolve(JSON.parse(f.toString()))); this.sock.write(Buffer.concat([head, body])); });
  }
  async request(messageId: number, message: Uint8Array): Promise<Uint8Array> {
    this.requests.push(messageId);
    let last: Record<string, any> = {};
    for (const f of fragment(messageId, message, 20)) {
      last = await this.call('gatt.request', {peer: this.peer, fragment_hex: toHex(f)});
      if (!last.ok) throw new ProvisioningError(last.error.code);
    }
    if (this.dropAt-- === 0) throw new ProvisioningError('response_timeout');
    return fromHex(last.result.response_hex);
  }
  async reconnect() {
    // A BLE reconnect may come back with a new (random) address.
    await this.call('gatt.disconnect', {peer: this.peer});
    this.peer = `${this.peer}-r${++this.reconnects}`;
  }
}

describe.skipIf(!bin)('interop with Rust t3box-provision (protocol v2)', () => {
  let dir: string, child: ChildProcess, sock: Socket;
  beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), 't3box-ts-'));
    child = spawn(bin!, [], {env: {...process.env, T3BOX_PROVISION_SOCKET: join(dir, 'p.sock'), T3BOX_CREDENTIALS_DIR: dir,
      T3BOX_SETUP_FILE: join(dir, 'none'), T3BOX_NETWORK_BACKEND: 'virtual'}, stdio: 'ignore'});
    for (let i = 0; i < 100 && !existsSync(join(dir, 'p.sock')); i++) await new Promise(r => setTimeout(r, 30));
    sock = await new Promise<Socket>((resolve, reject) => { const s = connect(join(dir, 'p.sock'), () => resolve(s)); s.on('error', reject); });
  });
  afterAll(() => { sock?.destroy(); child?.kill(); rmSync(dir, {recursive: true, force: true}); });

  it('claims, scans and joins with sealed credentials; lost responses are replayed', async () => {
    const t = new SocketTransport(sock, 'AA:00:00:00:00:01');
    const session = new ProvisioningSession(t);
    expect((await session.deviceInformation()).protocol).toBe(2);
    const secret = fromHex(readFileSync(join(dir, 'setup-secret'), 'utf8').trim());
    await expect(session.claim(new Uint8Array(16), 'ts-phone')).rejects.toMatchObject({code: 'bad_proof'});

    // The claim succeeds on the device but its response is lost (request #1
    // after the challenge). The session reconnects and resends the identical
    // request; the device replays CLAIMED instead of answering not_claimable.
    let before = t.requests.length;
    t.loseResponseOf(1);
    const key = await session.claim(secret, 'ts-phone');
    expect(key.length).toBe(32);
    let ids = t.requests.slice(before);
    expect(ids).toHaveLength(3);
    expect(ids[1]).toBe(ids[2]);
    expect(t.reconnects).toBe(1);

    // Same for the scan itself (request #1 after the operation challenge).
    before = t.requests.length;
    t.loseResponseOf(1);
    expect(await session.scan()).toEqual([{ssid: 'virtual-lab', rssi: -40, security: 'wpa-psk'}]);
    ids = t.requests.slice(before);
    expect(ids[1]).toBe(ids[2]);

    expect((await session.deviceInformation()).ownership).toBe('CLAIMED');
    await expect(session.claim(secret, 'intruder')).rejects.toMatchObject({code: 'already_claimed'});
    expect(await session.configureWifi({ssid: 'elsewhere', security: 'wpa-psk', psk: 'password1'})).toBe('FAILED');
    expect((await session.networkStatus()).last_error).toBe('join_failed');
    t.loseResponseOf(1);
    expect(await session.configureWifi({ssid: 'virtual-lab', security: 'wpa-psk', psk: 'password1'})).toBe('CONNECTED');
    expect(t.reconnects).toBe(3);
    const other = new ProvisioningSession(new SocketTransport(sock, 'BB:00:00:00:00:02'));
    other.useOwnerKey(new Uint8Array(32));
    await expect(other.scan()).rejects.toMatchObject({code: 'bad_proof'});
  });

  it('does not retry non-retriable device errors', async () => {
    const t = new SocketTransport(sock, 'CC:00:00:00:00:03');
    const s = new ProvisioningSession(t);
    await expect(s.call({op: 'nope'})).rejects.toMatchObject({code: 'unknown_op'});
    expect(t.requests.length).toBe(1);
  });
});

