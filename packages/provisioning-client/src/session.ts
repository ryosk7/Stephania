// Phone-side provisioning flow, protocol v3 (contracts/ble-protocol.md).
// One request/response channel; the transport carries a complete request
// message and returns the complete response (fragmentation, indications and
// reassembly are the transport's job).
import {claimConfirmation, deriveOwnerKey, fromHex, open, operationProof, OWNER_ID, seal, toHex, x25519PublicKey, x25519Shared} from './crypto';

export const UUID = {
  service: '7c3b0200-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  info: '7c3b0201-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  rpc: '7c3b0202-5d1e-4a6f-9b0e-3a1f2c4d5e60',
} as const;

export const PROTOCOL_VERSION = 3;

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
/**
 * `otpSecret` (base64, 20 bytes) is the maintenance code secret
 * (docs/MAINTENANCE.md): absent when the device has no maintenance service.
 * Each enrollment rotates it, so only the latest phone's codes work.
 */
export type ManagementEnrollment = {token: string; tokenId: string; urls: string[]; caSha256: string; otpSecret?: string};

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
  // Start at a random id so a new session does not repeat the ids of an
  // earlier one (the device's replay cache is keyed by id and bytes).
  private messageId = Math.floor(Math.random() * 255);
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
   * Become the owner of an unclaimed device: the first claim wins. An X25519
   * exchange gives both sides the owner key without sending it; the app
   * keeps it in secure storage. `privateKey` is 32 fresh random bytes (the
   * caller supplies them so React Native can use its native random source).
   *
   * `beforeClaim` receives the derived owner key before the claim is sent,
   * so the caller can store it as pending: if the device stores the claim
   * but its response is lost, the phone still holds the key.
   */
  async claim(ownerId: string, privateKey: Uint8Array, beforeClaim?: (ownerKey: Uint8Array) => Promise<void>): Promise<Uint8Array> {
    if (!OWNER_ID.test(ownerId)) throw new ProvisioningError('invalid_owner_id');
    if (privateKey.length !== 32) throw new ProvisioningError('invalid_key');
    const challenge = await this.call({op: 'pairing.challenge'});
    if (challenge.state !== 'PAIRING' || typeof challenge.nonce !== 'string' || typeof challenge.device_key !== 'string') {
      throw new ProvisioningError(challenge.state === 'CLAIMED' ? 'already_claimed' : 'not_claimable');
    }
    const nonce = fromHex(challenge.nonce), deviceKey = fromHex(challenge.device_key);
    const phoneKey = x25519PublicKey(privateKey);
    const ownerKey = deriveOwnerKey(x25519Shared(privateKey, deviceKey), nonce, ownerId, deviceKey, phoneKey);
    await beforeClaim?.(ownerKey);
    const r = await this.call({op: 'pairing.claim', owner_id: ownerId, phone_key: toHex(phoneKey)});
    if (r.state !== 'CLAIMED' || r.confirm !== toHex(claimConfirmation(ownerKey, nonce))) throw new ProvisioningError('claim_failed');
    this.ownerKey = ownerKey;
    return ownerKey;
  }

  /** Whether the device has a working cable connection (Wi-Fi setup can be skipped). */
  async wired(): Promise<boolean> {
    return (await this.networkStatus()).wired === true;
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
    const enrollment: ManagementEnrollment = {token: v.token, tokenId: v.token_id, urls, caSha256: v.ca_sha256};
    if (v.otp_secret !== undefined) {
      if (typeof v.otp_secret !== 'string' || !/^[A-Za-z0-9+/]{26}[AEIMQUYcgkosw048]=$/.test(v.otp_secret)) throw new ProvisioningError('invalid_response');
      enrollment.otpSecret = v.otp_secret;
    }
    return enrollment;
  }
}
