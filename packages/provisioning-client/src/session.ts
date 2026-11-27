// Phone-side provisioning flow, protocol v2 (contracts/ble-protocol.md).
// One request/response channel; the transport carries a complete request
// message and returns the complete response (fragmentation, indications and
// reassembly are the transport's job).
import {claimProof, deriveOwnerKey, fromHex, open, operationProof, OWNER_ID, seal, toHex} from './crypto';

export const UUID = {
  service: '7c3b0100-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  info: '7c3b0101-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  rpc: '7c3b0102-5d1e-4a6f-9b0e-3a1f2c4d5e60',
} as const;

export const PROTOCOL_VERSION = 2;

/** Implemented by react-native-ble-plx in the app and by a socket in tests. */
export interface BleTransport {
  /** Send one complete request under `messageId`; resolve with the complete response. */
  request(messageId: number, message: Uint8Array, timeoutMs: number): Promise<Uint8Array>;
  /** Re-establish the link after a drop (optional). */
  reconnect?(): Promise<void>;
}

export class ProvisioningError extends Error {
  constructor(readonly code: string) { super(code); }
}

/** Transport failures after which an identical retry is safe (device replays). */
const RETRIABLE = new Set(['response_timeout', 'ble_write_failed', 'ble_notify_failed', 'disconnected', 'in_progress']);

export type WifiConfig = {ssid: string; security: 'open'} | {ssid: string; security: 'wpa-psk'; psk: string};
export type ScanNetwork = {ssid: string; rssi: number; security: string};
/** LAN management access delivered sealed by the device. Keep `token` in
 * secure storage; connect only to servers whose chain ends in `caSha256`. */
export type ManagementEnrollment = {token: string; tokenId: string; urls: string[]; caSha256: string};

const dec = new TextDecoder();
const enc = new TextEncoder();

export function validateWifi(c: WifiConfig): void {
  const ssidBytes = enc.encode(c.ssid).length;
  if (ssidBytes < 1 || ssidBytes > 32) throw new ProvisioningError('invalid_ssid');
  if (c.security === 'wpa-psk') {
    const printable = c.psk.length >= 8 && c.psk.length <= 63 && /^[\x20-\x7e]+$/.test(c.psk);
    const raw = /^[0-9a-fA-F]{64}$/.test(c.psk);
    if (!printable && !raw) throw new ProvisioningError('invalid_psk');
  }
}

const TIMEOUTS: Record<string, number> = {'wifi.configure': 45_000, 'wifi.scan': 20_000};

export class ProvisioningSession {
  private messageId = 0;
  private ownerKey?: Uint8Array;

  constructor(private readonly transport: BleTransport, private readonly retries = 2) {}

  /** Restore a previously derived owner key from secure storage (Keychain/Keystore). */
  useOwnerKey(key: Uint8Array) { this.ownerKey = key; }

  /**
   * Send a request. On a transport failure the *identical* bytes are resent
   * under the same message id (after reconnecting if possible); the device
   * replays the original response instead of running the operation twice.
   */
  async call(body: Record<string, unknown>): Promise<Record<string, unknown>> {
    this.messageId = (this.messageId % 255) + 1;
    const id = this.messageId;
    const bytes = enc.encode(JSON.stringify(body));
    const timeout = TIMEOUTS[String(body.op)] ?? 15_000;
    for (let attempt = 0; ; attempt++) {
      try {
        const r = JSON.parse(dec.decode(await this.transport.request(id, bytes, timeout))) as Record<string, unknown>;
        if (typeof r.error === 'string') throw new ProvisioningError(r.error);
        return r;
      } catch (e) {
        const code = e instanceof ProvisioningError ? e.code : 'transport_error';
        if (!RETRIABLE.has(code) || attempt >= this.retries) throw e;
        await this.transport.reconnect?.();
      }
    }
  }

  deviceInformation() { return this.call({op: 'device.info'}); }
  networkStatus() { return this.call({op: 'network.status'}); }

  /**
   * Claim with the setup secret (from the setup QR). Returns the owner key,
   * which the app must keep in secure storage; it is never transmitted.
   */
  async claim(setupSecret: Uint8Array, ownerId: string): Promise<Uint8Array> {
    if (setupSecret.length !== 16) throw new ProvisioningError('invalid_setup_secret');
    if (!OWNER_ID.test(ownerId)) throw new ProvisioningError('invalid_owner_id');
    const challenge = await this.call({op: 'pairing.challenge'});
    if (challenge.state !== 'PAIRING' || typeof challenge.nonce !== 'string') {
      throw new ProvisioningError(challenge.state === 'CLAIMED' ? 'already_claimed' : 'not_claimable');
    }
    const nonce = fromHex(challenge.nonce);
    const r = await this.call({op: 'pairing.claim', owner_id: ownerId, proof: toHex(claimProof(setupSecret, nonce, ownerId))});
    if (r.state !== 'CLAIMED') throw new ProvisioningError('claim_failed');
    this.ownerKey = deriveOwnerKey(setupSecret, nonce, ownerId);
    return this.ownerKey;
  }

  private async operationNonce(): Promise<Uint8Array> {
    if (!this.ownerKey) throw new ProvisioningError('not_owner');
    const c = await this.call({op: 'operation.challenge'});
    if (typeof c.nonce !== 'string') throw new ProvisioningError('no_challenge');
    return fromHex(c.nonce);
  }

  async scan(): Promise<ScanNetwork[]> {
    const nonce = await this.operationNonce();
    const r = await this.call({op: 'wifi.scan', proof: toHex(operationProof(this.ownerKey!, nonce, 'wifi.scan', new Uint8Array()))});
    return (r.networks as ScanNetwork[] | undefined) ?? [];
  }

  /** The configuration (with PSK) is sealed; never log `config`. */
  async configureWifi(config: WifiConfig): Promise<string> {
    validateWifi(config);
    const nonce = await this.operationNonce();
    const sealed = seal(this.ownerKey!, nonce, 'wifi.configure', enc.encode(JSON.stringify(config)));
    const r = await this.call({op: 'wifi.configure', sealed: toHex(sealed), proof: toHex(operationProof(this.ownerKey!, nonce, 'wifi.configure', sealed))});
    if (typeof r.state !== 'string') throw new ProvisioningError('invalid_response');
    return r.state;
  }

  /** Whether the device has LAN TLS set up (unauthenticated hint only). */
  async managementAvailable(): Promise<boolean> {
    return (await this.call({op: 'management.endpoint'})).available === true;
  }

  /**
   * Owner-only: issues a LAN access token. URLs and the CA pin arrive
   * sealed under the owner key, so a BLE man-in-the-middle cannot swap them.
   */
  async enrollManagement(): Promise<ManagementEnrollment> {
    const nonce = await this.operationNonce();
    const r = await this.call({op: 'management.enroll', proof: toHex(operationProof(this.ownerKey!, nonce, 'management.enroll', new Uint8Array()))});
    if (typeof r.sealed !== 'string') throw new ProvisioningError('invalid_response');
    const plain = open(this.ownerKey!, nonce, 'management.enroll', fromHex(r.sealed));
    if (!plain) throw new ProvisioningError('bad_seal');
    const v = JSON.parse(dec.decode(plain)) as Record<string, unknown>;
    const urls = Array.isArray(v.urls) ? v.urls.filter((u): u is string => typeof u === 'string' && u.startsWith('https://')) : [];
    if (typeof v.token !== 'string' || !/^t3b_[0-9a-f]{64}$/.test(v.token) || typeof v.token_id !== 'string' ||
        typeof v.ca_sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(v.ca_sha256) || urls.length === 0) {
      throw new ProvisioningError('invalid_response');
    }
    return {token: v.token, tokenId: v.token_id, urls, caSha256: v.ca_sha256};
  }
}
