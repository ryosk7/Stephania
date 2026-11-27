// BleTransport over react-native-ble-plx (contracts/ble-protocol.md).
// Writes are fragmented to the negotiated ATT payload; the response to a
// complete write arrives as notifications on the same characteristic under
// the request's message id, which this transport reassembles.
import {ProvisioningError, Reassembler, UUID, type BleTransport} from '@t3box/provisioning-client';
import {fromBase64, toBase64} from './base64';

/** The subset of BleManager this transport needs (keeps it unit-testable). */
export interface BleLike {
  readCharacteristicForDevice(deviceId: string, service: string, characteristic: string): Promise<{value: string | null}>;
  writeCharacteristicWithResponseForDevice(deviceId: string, service: string, characteristic: string, base64: string): Promise<unknown>;
  monitorCharacteristicForDevice(
    deviceId: string, service: string, characteristic: string,
    listener: (error: Error | null, c: {value: string | null} | null) => void,
  ): {remove(): void};
}

const WRITE_TIMEOUT_MS: Record<string, number> = {
  // NetworkManager may wait up to 30 s to join.
  [UUID.wifiConfiguration]: 45_000,
  [UUID.wifiScanRequest]: 20_000,
};

export class BlePlxTransport implements BleTransport {
  constructor(private readonly ble: BleLike, private readonly deviceId: string, readonly attPayload: number) {
    if (attPayload < 20) throw new ProvisioningError('mtu_too_small');
  }

  async read(uuid: string): Promise<Uint8Array> {
    try {
      const c = await this.ble.readCharacteristicForDevice(this.deviceId, UUID.service, uuid);
      return fromBase64(c.value ?? '');
    } catch {
      throw new ProvisioningError('ble_read_failed');
    }
  }

  /**
   * Called by ProvisioningSession once per fragment. The session sends all
   * fragments of one message with the same id; we subscribe on the first
   * fragment and resolve with the reassembled response after the last.
   */
  private pending?: {id: number; response: Promise<Uint8Array>; cancel(): void};

  async write(uuid: string, frag: Uint8Array): Promise<Uint8Array | null> {
    const [id, index, count] = frag;
    if (index === 0) {
      this.pending?.cancel();
      this.pending = this.subscribe(uuid, id);
    }
    const pending = this.pending;
    if (!pending || pending.id !== id) throw new ProvisioningError('out_of_order_write');
    try {
      await this.ble.writeCharacteristicWithResponseForDevice(this.deviceId, UUID.service, uuid, toBase64(frag));
    } catch {
      pending.cancel();
      throw new ProvisioningError('ble_write_failed');
    }
    if (index + 1 < count) return null;
    try {
      return await pending.response;
    } finally {
      pending.cancel();
      if (this.pending === pending) this.pending = undefined;
    }
  }

  private subscribe(uuid: string, id: number) {
    const r = new Reassembler(id);
    let settle!: {resolve: (b: Uint8Array) => void; reject: (e: Error) => void};
    const response = new Promise<Uint8Array>((resolve, reject) => { settle = {resolve, reject}; });
    response.catch(() => undefined); // avoid unhandled rejection if never awaited
    const sub = this.ble.monitorCharacteristicForDevice(this.deviceId, UUID.service, uuid, (error, c) => {
      if (error) return settle.reject(new ProvisioningError('ble_notify_failed'));
      if (!c?.value) return;
      try {
        const done = r.push(fromBase64(c.value));
        if (done) settle.resolve(done);
      } catch (e) {
        settle.reject(new ProvisioningError(e instanceof Error ? e.message : 'malformed'));
      }
    });
    const timer = setTimeout(() => settle.reject(new ProvisioningError('response_timeout')), WRITE_TIMEOUT_MS[uuid] ?? 15_000);
    return {id, response, cancel: () => { clearTimeout(timer); sub.remove(); }};
  }
}
