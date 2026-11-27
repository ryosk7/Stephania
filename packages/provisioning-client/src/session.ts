// Phone-side provisioning flow over an abstract BLE transport.
import {claimProof, deriveOwnerKey, fromHex, operationProof, OWNER_ID, toHex} from './crypto';
import {fragment} from './framing';

export const UUID = {
  service: '7c3b0001-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  protocolVersion: '7c3b0002-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  deviceInformation: '7c3b0003-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  pairingChallenge: '7c3b0004-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  pairingResponse: '7c3b0005-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  wifiScanRequest: '7c3b0006-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  wifiScanResults: '7c3b0007-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  wifiConfiguration: '7c3b0008-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  networkStatus: '7c3b0009-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  managementEndpoint: '7c3b000a-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  devicePublicKey: '7c3b000b-5d1e-4a6f-9b0e-3a1f2c4d5e60',
  operationChallenge: '7c3b000c-5d1e-4a6f-9b0e-3a1f2c4d5e60',
} as const;

/** Implemented by react-native-ble-plx in the app and by a socket in tests. */
export interface BleTransport {
  /** Negotiated ATT payload (MTU - 3). */
  readonly attPayload: number;
  read(uuid: string): Promise<Uint8Array>;
  /** Write one fragment; resolves with the device response once the message is complete. */
  write(uuid: string, fragment: Uint8Array): Promise<Uint8Array | null>;
}

export class ProvisioningError extends Error {
  constructor(readonly code: string) { super(code); }
}

export type WifiConfig = {ssid: string; security: 'open'} | {ssid: string; security: 'wpa-psk'; psk: string};
export type ScanNetwork = {ssid: string; rssi: number; security: string};

const dec = new TextDecoder();
const enc = new TextEncoder();
const json = (b: Uint8Array) => JSON.parse(dec.decode(b)) as Record<string, unknown>;

export function validateWifi(c: WifiConfig): void {
  const ssidBytes = enc.encode(c.ssid).length;
  if (ssidBytes < 1 || ssidBytes > 32) throw new ProvisioningError('invalid_ssid');
  if (c.security === 'wpa-psk') {
    const printable = c.psk.length >= 8 && c.psk.length <= 63 && /^[\x20-\x7e]+$/.test(c.psk);
    const raw = /^[0-9a-fA-F]{64}$/.test(c.psk);
    if (!printable && !raw) throw new ProvisioningError('invalid_psk');
  }
}

export class ProvisioningSession {
  private messageId = 0;
  private ownerKey?: Uint8Array;

  constructor(private readonly transport: BleTransport) {}

  /** Restore a previously derived owner key from secure storage (Keychain/Keystore). */
  useOwnerKey(key: Uint8Array) { this.ownerKey = key; }

  async deviceInformation() { return json(await this.transport.read(UUID.deviceInformation)); }

  private async send(uuid: string, body: unknown): Promise<Record<string, unknown>> {
    this.messageId = (this.messageId + 1) & 0xff;
    let response: Uint8Array | null = null;
    for (const f of fragment(this.messageId, enc.encode(JSON.stringify(body)), this.transport.attPayload)) {
      response = await this.transport.write(uuid, f);
    }
    if (!response) throw new ProvisioningError('no_response');
    const r = json(response);
    // Over BLE, device errors arrive as a notified {"error": code}.
    if (typeof r.error === 'string') throw new ProvisioningError(r.error);
    return r;
  }

  /**
   * Claim with the setup secret (from the device label/QR). Returns the owner
   * key, which the app must store in secure storage; it is never transmitted.
   */
  async claim(setupSecret: Uint8Array, ownerId: string): Promise<Uint8Array> {
    if (setupSecret.length !== 16) throw new ProvisioningError('invalid_setup_secret');
    if (!OWNER_ID.test(ownerId)) throw new ProvisioningError('invalid_owner_id');
    const challenge = json(await this.transport.read(UUID.pairingChallenge));
    if (challenge.state !== 'PAIRING' || typeof challenge.nonce !== 'string') {
      throw new ProvisioningError(challenge.state === 'CLAIMED' ? 'already_claimed' : 'not_claimable');
    }
    const nonce = fromHex(challenge.nonce);
    const r = await this.send(UUID.pairingResponse, {owner_id: ownerId, proof: toHex(claimProof(setupSecret, nonce, ownerId))});
    if (r.state !== 'CLAIMED') throw new ProvisioningError('claim_failed');
    this.ownerKey = deriveOwnerKey(setupSecret, nonce, ownerId);
    return this.ownerKey;
  }

  private async operation(uuid: string, operation: string, payload: Uint8Array) {
    if (!this.ownerKey) throw new ProvisioningError('not_owner');
    const c = json(await this.transport.read(UUID.operationChallenge));
    if (typeof c.nonce !== 'string') throw new ProvisioningError('no_challenge');
    const proof = operationProof(this.ownerKey, fromHex(c.nonce), operation, payload);
    return this.send(uuid, {operation, payload_hex: toHex(payload), proof: toHex(proof)});
  }

  async scan(): Promise<ScanNetwork[]> {
    const r = await this.operation(UUID.wifiScanRequest, 'wifi.scan', enc.encode('{}'));
    const pages = typeof r.pages === 'number' ? r.pages : 0;
    const out: ScanNetwork[] = [];
    for (let i = 0; i < pages; i++) {
      const page = json(await this.transport.read(UUID.wifiScanResults));
      out.push(...(page.networks as ScanNetwork[]));
    }
    return out;
  }

  /** Never log `config`; it carries the PSK. */
  async configureWifi(config: WifiConfig): Promise<string> {
    validateWifi(config);
    const r = await this.operation(UUID.wifiConfiguration, 'wifi.configure', enc.encode(JSON.stringify(config)));
    if (typeof r.state !== 'string') throw new ProvisioningError('invalid_response');
    return r.state;
  }

  async networkStatus() { return json(await this.transport.read(UUID.networkStatus)); }
}
