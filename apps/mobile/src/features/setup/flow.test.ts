// The setup flow's owner-key handling against the real Rust device core:
// a key this phone already holds is never replaced just because a device
// reports UNCLAIMED, and a claim whose answer is lost leaves the phone with
// the key the device stored.
import {spawn, type ChildProcess} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync} from 'node:fs';
import {connect, type Socket} from 'node:net';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {afterEach, describe, expect, it, vi} from 'vitest';
import {fragment, fromHex, ProvisioningError, toHex, type BleTransport} from '@stephania/provisioning-client';

const bin = process.env.STEPHANIA_PROVISION_BIN;

const store = new Map<string, string>();
vi.mock('expo-secure-store', () => ({
  WHEN_UNLOCKED_THIS_DEVICE_ONLY: 'unlocked-this-device',
  getItemAsync: async (k: string) => store.get(k) ?? null,
  setItemAsync: async (k: string, v: string) => { store.set(k, v); },
  deleteItemAsync: async (k: string) => { store.delete(k); },
}));
vi.mock('expo-crypto', () => ({getRandomBytes: (n: number) => new Uint8Array(randomBytes(n))}));
vi.mock('expo-haptics', () => ({notificationAsync: async () => {}, NotificationFeedbackType: {Success: 'success'}}));
const enrolled: string[] = [];
vi.mock('@/shared/device', () => ({saveEnrollment: async (id: string) => { enrolled.push(id); }}));

/** The device socket, playing the BLE adapter; can lose responses. */
/** `lose`: how many answers to `loseOp` requests to drop. */
const link = {sock: undefined as Socket | undefined, lose: 0, loseOp: '', requests: [] as string[]};
class SocketTransport implements BleTransport {
  private buf = Buffer.alloc(0);
  private waiting: ((b: Buffer) => void)[] = [];
  private n = 0;
  private peer = 'AA:00:00:00:00:01';
  constructor(private readonly sock: Socket) {
    sock.removeAllListeners('data');
    sock.on('data', (d: Buffer) => {
      this.buf = Buffer.concat([this.buf, d]);
      while (this.buf.length >= 4 && this.buf.length >= 4 + this.buf.readUInt32BE(0)) {
        const len = this.buf.readUInt32BE(0), f = this.buf.subarray(4, 4 + len);
        this.buf = this.buf.subarray(4 + len);
        this.waiting.shift()?.(f);
      }
    });
  }
  private call(method: string, params: object): Promise<Record<string, any>> {
    const body = Buffer.from(JSON.stringify({version: 1, request_id: 'f' + ++this.n, method, params}));
    const head = Buffer.alloc(4); head.writeUInt32BE(body.length);
    return new Promise(resolve => { this.waiting.push(f => resolve(JSON.parse(f.toString()))); this.sock.write(Buffer.concat([head, body])); });
  }
  async request(messageId: number, message: Uint8Array): Promise<Uint8Array> {
    const op = JSON.parse(new TextDecoder().decode(message)).op as string;
    link.requests.push(op);
    let last: Record<string, any> = {};
    for (const f of fragment(messageId, message, 20)) last = await this.call('gatt.request', {peer: this.peer, fragment_hex: toHex(f)});
    if (op === link.loseOp && link.lose > 0) { link.lose--; throw new ProvisioningError('response_timeout'); }
    return fromHex(last.result.response_hex);
  }
  async reconnect() {
    await this.call('gatt.disconnect', {peer: this.peer});
    this.peer = this.peer.replace(/..$/, m => String(Number(m) + 1).padStart(2, '0'));
  }
}
vi.mock('./ble/manager', () => ({
  connectTransport: async () => new SocketTransport(link.sock!),
  disconnect: async () => {},
}));

const {finishSetup, openDevice} = await import('./flow');
const state = await import('./state');

let dir = '', child: ChildProcess | undefined;
async function freshDevice() {
  dir = mkdtempSync(join(tmpdir(), 'stephania-flow-'));
  child = spawn(bin!, [], {env: {...process.env, STEPHANIA_PROVISION_SOCKET: join(dir, 'p.sock'), STEPHANIA_CREDENTIALS_DIR: dir,
    STEPHANIA_NETWORK_BACKEND: 'virtual', STEPHANIA_PKI_DIR: join(dir, 'pki'), STEPHANIA_MANAGEMENT_TOKENS: join(dir, 'tokens.json')}, stdio: 'ignore'});
  for (let i = 0; i < 100 && !existsSync(join(dir, 'p.sock')); i++) await new Promise(r => setTimeout(r, 30));
  link.sock = await new Promise<Socket>((resolve, reject) => { const s = connect(join(dir, 'p.sock'), () => resolve(s)); s.on('error', reject); });
  link.lose = 0; link.requests = [];
}
afterEach(() => {
  link.sock?.destroy(); child?.kill(); state.setActive(undefined); store.clear();
  if (dir) rmSync(dir, {recursive: true, force: true});
});

describe.skipIf(!bin)('setup flow owner keys (real device core)', () => {
  it('does not replace a key this phone holds unless the person says so', async () => {
    await freshDevice();
    const old = new Uint8Array(32).fill(7);
    await state.saveOwnerKey('dev-1', old);
    await expect(openDevice('dev-1', 'Stephania-0001')).rejects.toMatchObject({code: 'owner_key_exists'});
    expect(link.requests).not.toContain('pairing.claim');
    expect(await state.loadOwnerKey('dev-1')).toEqual(old);
    // Explicit: "I rewrote its card".
    await openDevice('dev-1', 'Stephania-0001', {replaceOwnerKey: true});
    const key = await state.loadOwnerKey('dev-1');
    expect(key).not.toEqual(old);
    expect(await state.loadPendingOwnerKey('dev-1')).toBeNull();
    expect(await state.activeSession()!.session.scan()).toHaveLength(1);
  });

  it('keeps the key of a claim whose answer was lost and promotes it once proved', async () => {
    await freshDevice();
    // The device stores the claim, but every answer to it is lost.
    link.loseOp = 'pairing.claim';
    link.lose = 3;
    await expect(openDevice('dev-2', 'Stephania-0002')).rejects.toMatchObject({code: 'response_timeout'});
    expect(link.requests.filter(r => r === 'pairing.claim')).toHaveLength(3);
    expect(await state.loadOwnerKey('dev-2')).toBeNull();
    const pending = await state.loadPendingOwnerKey('dev-2');
    expect(pending).not.toBeNull();
    // The device kept the claim; the next attempt opens it with that key.
    link.lose = 0;
    await openDevice('dev-2', 'Stephania-0002');
    expect(state.activeSession()!.pendingKey).toEqual(pending);
    mkdirSync(join(dir, 'pki'));
    writeFileSync(join(dir, 'pki', 'endpoint.json'), JSON.stringify({host: 'stephania.local', port: 9443, ips: [], urls: [], ca_sha256: 'ab'.repeat(32), not_after_unix: 2e9}));
    expect(await finishSetup()).toBe('dev-2');
    expect(enrolled).toContain('dev-2');
    expect(await state.loadOwnerKey('dev-2')).toEqual(pending);
    expect(await state.loadPendingOwnerKey('dev-2')).toBeNull();
  });
});
