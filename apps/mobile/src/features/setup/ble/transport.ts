// BleTransport over react-native-ble-plx, protocol v3 (contracts/ble-protocol.md).
// - One long-lived indication subscription on the RPC characteristic,
//   established before any write; indications are routed by message id, so
//   stale responses from an earlier attempt are ignored.
// - Requests are written as fragments with response (ATT acknowledged).
// - reconnect() re-establishes the link and subscription; the session then
//   resends the identical request, which the device replays.
import {fragment, ProvisioningError, Reassembler, UUID, type BleTransport} from '@stephania/provisioning-client';
import {fromBase64, toBase64} from './base64';

/** The subset of BleManager this transport needs (keeps it unit-testable). */
export interface BleLike {
  writeCharacteristicWithResponseForDevice(deviceId: string, service: string, characteristic: string, base64: string): Promise<unknown>;
  monitorCharacteristicForDevice(
    deviceId: string, service: string, characteristic: string,
    listener: (error: Error | null, c: {value: string | null} | null) => void,
  ): {remove(): void};
}

type Pending = {reassembler: Reassembler; resolve: (b: Uint8Array) => void; reject: (e: Error) => void};

export class BlePlxTransport implements BleTransport {
  private subscription?: {remove(): void};
  private pending = new Map<number, Pending>();

  constructor(
    private readonly ble: BleLike,
    private readonly deviceId: string,
    readonly attPayload: number,
    private readonly reconnectLink?: () => Promise<void>,
  ) {
    if (attPayload < 20) throw new ProvisioningError('mtu_too_small');
  }

  private subscribe() {
    if (this.subscription) return;
    this.subscription = this.ble.monitorCharacteristicForDevice(this.deviceId, UUID.service, UUID.rpc, (error, c) => {
      if (error) {
        // Link dropped or subscription lost: fail everything in flight.
        for (const p of this.pending.values()) p.reject(new ProvisioningError('disconnected'));
        this.pending.clear();
        this.subscription = undefined;
        return;
      }
      if (!c?.value) return;
      const frag = fromBase64(c.value);
      const p = this.pending.get(frag[0]);
      if (!p) return; // stale response from an abandoned attempt
      try {
        const done = p.reassembler.push(frag);
        if (done) { this.pending.delete(frag[0]); p.resolve(done); }
      } catch {
        // Identical duplicates are ignored by the reassembler; anything
        // reaching here is a real inconsistency.
        this.pending.delete(frag[0]);
        p.reject(new ProvisioningError('malformed_response'));
      }
    });
  }

  async request(messageId: number, message: Uint8Array, timeoutMs: number): Promise<Uint8Array> {
    this.subscribe();
    this.pending.get(messageId)?.reject(new ProvisioningError('superseded'));
    const response = new Promise<Uint8Array>((resolve, reject) => {
      this.pending.set(messageId, {reassembler: new Reassembler(messageId), resolve, reject});
    });
    const timer = setTimeout(() => {
      const p = this.pending.get(messageId);
      if (p) { this.pending.delete(messageId); p.reject(new ProvisioningError('response_timeout')); }
    }, timeoutMs);
    try {
      for (const f of fragment(messageId, message, this.attPayload)) {
        try {
          await this.ble.writeCharacteristicWithResponseForDevice(this.deviceId, UUID.service, UUID.rpc, toBase64(f));
        } catch {
          this.pending.delete(messageId);
          throw new ProvisioningError('ble_write_failed');
        }
      }
      return await response;
    } finally {
      clearTimeout(timer);
    }
  }

  async reconnect(): Promise<void> {
    this.subscription?.remove();
    this.subscription = undefined;
    await this.reconnectLink?.();
    this.subscribe();
  }

  close() {
    this.subscription?.remove();
    this.subscription = undefined;
    for (const p of this.pending.values()) p.reject(new ProvisioningError('closed'));
    this.pending.clear();
  }
}
