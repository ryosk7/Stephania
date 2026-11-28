// TypeScript phone client (protocol v3) against the real Rust stephania-provision
// binary. Enabled when STEPHANIA_PROVISION_BIN points at a ci-virtual build.
import {spawn, type ChildProcess} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync} from 'node:fs';
import {connect, type Socket} from 'node:net';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {afterAll, beforeAll, describe, expect, it} from 'vitest';
import {fragment, fromHex, otpSecretBytes, ProvisioningError, ProvisioningSession, toHex, totpCode, type BleTransport} from './index';

const bin = process.env.STEPHANIA_PROVISION_BIN;
// Optional: the real maintenance service, so enrollment rotates and delivers
// its code secret (docs/MAINTENANCE.md).
const maintBin = process.env.STEPHANIA_MAINT_BIN;

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

/** One request to stephania-maint's socket (docs/IPC.md framing). */
function maintRpc(path: string, method: string, params: object): Promise<Record<string, any>> {
  return new Promise((resolve, reject) => {
    const s = connect(path, () => {
      const body = Buffer.from(JSON.stringify({version: 1, request_id: 'ts', method, params}));
      const head = Buffer.alloc(4); head.writeUInt32BE(body.length);
      s.write(Buffer.concat([head, body]));
    });
    let buf = Buffer.alloc(0);
    s.on('data', d => {
      buf = Buffer.concat([buf, d]);
      if (buf.length >= 4 && buf.length >= 4 + buf.readUInt32BE(0)) { s.destroy(); resolve(JSON.parse(buf.subarray(4, 4 + buf.readUInt32BE(0)).toString())); }
    });
    s.on('error', reject);
  });
}

describe.skipIf(!bin)('interop with Rust stephania-provision (protocol v3)', () => {
  let dir: string, child: ChildProcess, maint: ChildProcess | undefined, sock: Socket;
  beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), 'stephania-ts-'));
    const maintSocket = maintBin ? join(dir, 'm.sock') : '';
    if (maintBin) {
      maint = spawn(maintBin, [], {env: {...process.env, STEPHANIA_MAINT_DIR: join(dir, 'maint'), STEPHANIA_MAINT_SOCKET: maintSocket,
        STEPHANIA_PKI_DIR: join(dir, 'pki'), STEPHANIA_MAINT_SYSTEMCTL: 'true'}, stdio: 'ignore'});
      for (let i = 0; i < 100 && !existsSync(maintSocket); i++) await new Promise(r => setTimeout(r, 30));
    }
    child = spawn(bin!, [], {env: {...process.env, STEPHANIA_PROVISION_SOCKET: join(dir, 'p.sock'), STEPHANIA_CREDENTIALS_DIR: dir,
      STEPHANIA_NETWORK_BACKEND: 'virtual', STEPHANIA_PKI_DIR: join(dir, 'pki'),
      STEPHANIA_MANAGEMENT_TOKENS: join(dir, 'tokens.json'), STEPHANIA_MAINT_SOCKET: maintSocket}, stdio: 'ignore'});
    for (let i = 0; i < 100 && !existsSync(join(dir, 'p.sock')); i++) await new Promise(r => setTimeout(r, 30));
    sock = await new Promise<Socket>((resolve, reject) => { const s = connect(join(dir, 'p.sock'), () => resolve(s)); s.on('error', reject); });
  });
  afterAll(() => { sock?.destroy(); child?.kill(); maint?.kill(); rmSync(dir, {recursive: true, force: true}); });

  it('claims, scans and joins with sealed credentials; lost responses are replayed', async () => {
    const t = new SocketTransport(sock, 'AA:00:00:00:00:01');
    const session = new ProvisioningSession(t);
    expect((await session.deviceInformation()).protocol).toBe(3);
    expect(await session.wired()).toBe(false);
    const key32 = () => randomBytes(32);

    // The claim succeeds on the device but its response is lost (request #1
    // after the challenge). The session reconnects and resends the identical
    // request; the device replays CLAIMED instead of answering not_claimable.
    let before = t.requests.length;
    t.loseResponseOf(1);
    const key = await session.claim('ts-phone', key32());
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
    await expect(session.claim('intruder', key32())).rejects.toMatchObject({code: 'already_claimed'});
    expect(await session.configureWifi({ssid: 'elsewhere', security: 'wpa-psk', psk: 'password1'})).toBe('FAILED');
    expect((await session.networkStatus()).last_error).toBe('join_failed');
    t.loseResponseOf(1);
    expect(await session.configureWifi({ssid: 'virtual-lab', security: 'wpa-psk', psk: 'password1'})).toBe('CONNECTED');
    expect(t.reconnects).toBe(3);
    const other = new ProvisioningSession(new SocketTransport(sock, 'BB:00:00:00:00:02'));
    other.useOwnerKey(new Uint8Array(32));
    await expect(other.scan()).rejects.toMatchObject({code: 'bad_proof'});

    // LAN enrollment: fails closed until the device PKI has published an
    // endpoint, then delivers a sealed token + CA pin, replayed on loss.
    expect(await session.managementAvailable()).toBe(false);
    await expect(session.enrollManagement()).rejects.toMatchObject({code: 'management_unavailable'});
    mkdirSync(join(dir, 'pki'));
    const pin = 'ab'.repeat(32);
    writeFileSync(join(dir, 'pki', 'endpoint.json'), JSON.stringify({host: 'stephania.local', port: 9443, ips: ['192.168.1.2'], urls: [], ca_sha256: pin, not_after_unix: 2e9}));
    expect(await session.managementAvailable()).toBe(true);
    t.loseResponseOf(1);
    const e = await session.enrollManagement();
    expect(e).toMatchObject({urls: ['https://stephania.local:9443/', 'https://192.168.1.2:9443/'], caSha256: pin});
    expect(e.token).toMatch(/^t3b_[0-9a-f]{64}$/);
    const stored = JSON.parse(readFileSync(join(dir, 'tokens.json'), 'utf8')).tokens;
    expect(stored).toHaveLength(1);
    expect(JSON.stringify(stored)).not.toContain(e.token.slice(4));
    if (maintBin) {
      // The secret stephania-maint now verifies codes with, delivered sealed.
      const secret = JSON.parse(readFileSync(join(dir, 'maint', 'state.json'), 'utf8')).otp_secret as string;
      expect(e.otpSecret).toBe(Buffer.from(secret, 'hex').toString('base64'));
      // A code computed here (as Connect shows it) opens a window on the device.
      const code = totpCode(otpSecretBytes(e.otpSecret!), Date.now() / 1000);
      const opened = await maintRpc(join(dir, 'm.sock'), 'window.open', {actor: 'connect', client_addr: '127.0.0.1', code});
      expect(opened).toMatchObject({ok: true, result: {user: 'maint'}});
    } else {
      expect(e.otpSecret).toBeUndefined();
    }
    await expect(other.enrollManagement()).rejects.toMatchObject({code: 'bad_proof'});
  });

  it('never reuses a nonce when challenges are lost or ids repeat across sessions', async () => {
    // Fresh device: the claim above owns the shared one.
    const d = mkdtempSync(join(tmpdir(), 'stephania-ts2-'));
    const c = spawn(bin!, [], {env: {...process.env, STEPHANIA_PROVISION_SOCKET: join(d, 'p.sock'), STEPHANIA_CREDENTIALS_DIR: d,
      STEPHANIA_NETWORK_BACKEND: 'virtual'}, stdio: 'ignore'});
    try {
      for (let i = 0; i < 100 && !existsSync(join(d, 'p.sock')); i++) await new Promise(r => setTimeout(r, 30));
      const s2 = await new Promise<Socket>((resolve, reject) => { const s = connect(join(d, 'p.sock'), () => resolve(s)); s.on('error', reject); });
      // An attacker pre-caches pairing challenges under every message id.
      const attacker = new SocketTransport(s2, 'EE:00:00:00:00:01');
      for (let id = 0; id < 256; id++) await attacker.request(id, new TextEncoder().encode('{"op":"pairing.challenge"}'));
      // The owner's challenge response is lost too; the retry is fresh and
      // the key stored before the claim is the one the device holds.
      const t = new SocketTransport(s2, 'AA:00:00:00:00:09');
      t.loseResponseOf(0);
      let pending: Uint8Array | undefined;
      const owner = new ProvisioningSession(t);
      const key = await owner.claim('ts-phone', randomBytes(32), async k => { pending = k; });
      expect(pending).toEqual(key);
      expect(t.reconnects).toBe(1);
      // Two sessions that use the same message ids each get a fresh nonce.
      for (let i = 0; i < 2; i++) {
        const s = new ProvisioningSession(new SocketTransport(s2, `AA:00:00:00:01:0${i}`));
        (s as unknown as {messageId: number}).messageId = 40;
        s.useOwnerKey(key);
        expect(await s.scan()).toHaveLength(1);
      }
      s2.destroy();
    } finally {
      c.kill();
      rmSync(d, {recursive: true, force: true});
    }
  });

  it('does not retry non-retriable device errors', async () => {
    const t = new SocketTransport(sock, 'CC:00:00:00:00:03');
    const s = new ProvisioningSession(t);
    await expect(s.call({op: 'nope'})).rejects.toMatchObject({code: 'unknown_op'});
    expect(t.requests.length).toBe(1);
  });
});

