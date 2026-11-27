import {readFileSync} from 'node:fs';
import {join} from 'node:path';
import {describe, expect, it} from 'vitest';
import {claimConfirmation, deriveOwnerKey, fragment, fromHex, operationProof, seal, toHex, validateWifi, x25519PublicKey, x25519Shared, MAX_FRAGMENTS} from './index';

type Vector = Record<string, string>;
const vectors: Vector[] = JSON.parse(readFileSync(join(__dirname, '../../../contracts/fixtures/ble/claim-vectors.json'), 'utf8')).vectors;

describe('contracts/fixtures/ble/claim-vectors.json', () => {
  it.each(vectors.map(v => [v.owner_id, v] as [string, Vector]))('%s', (_: string, v: Vector) => {
    const nonce = fromHex(v.nonce);
    expect(toHex(x25519PublicKey(fromHex(v.phone_private)))).toBe(v.phone_key);
    const shared = x25519Shared(fromHex(v.phone_private), fromHex(v.device_key));
    expect(toHex(shared)).toBe(v.shared);
    const key = deriveOwnerKey(shared, nonce, v.owner_id, fromHex(v.device_key), fromHex(v.phone_key));
    expect(toHex(key)).toBe(v.owner_key);
    expect(toHex(claimConfirmation(key, nonce))).toBe(v.confirm);
    expect(toHex(operationProof(key, fromHex(v.operation_nonce), v.operation, fromHex(v.payload_hex)))).toBe(v.operation_proof);
  });
});

const sealVectors: Vector[] = JSON.parse(readFileSync(join(__dirname, '../../../contracts/fixtures/ble/seal-vectors.json'), 'utf8')).vectors;
describe('contracts/fixtures/ble/seal-vectors.json', () => {
  it('matches Python cryptography ChaCha20-Poly1305', () => {
    for (const v of sealVectors) {
      expect(toHex(seal(fromHex(v.owner_key), fromHex(v.operation_nonce), v.operation, fromHex(v.plaintext_hex)))).toBe(v.sealed_hex);
    }
  });
});

describe('framing', () => {
  it('matches device bounds', () => {
    const msg = new Uint8Array(4096).map((_, i) => i);
    // 255 fragments x 17 bytes cover a full 4 KiB message at the minimum MTU.
    const min = fragment(1, msg, 20);
    expect(min.length).toBe(241);
    expect(min.length).toBeLessThanOrEqual(MAX_FRAGMENTS);
    expect(() => fragment(1, new Uint8Array(4097), 509)).toThrow('too_large');
    const f = fragment(1, msg, 67);
    expect(f.length).toBe(64);
    expect(f.every(x => x.length <= 67)).toBe(true);
    expect(Array.from(f[5].subarray(0, 3))).toEqual([1, 5, 64]);
    expect(() => fragment(1, new Uint8Array(1), 19)).toThrow('payload_too_small');
    expect(() => fragment(1, new Uint8Array(0), 100)).toThrow('too_large');
  });
  it('validates wifi like the device', () => {
    expect(() => validateWifi({ssid: '', security: 'open'})).toThrow('invalid_ssid');
    expect(() => validateWifi({ssid: 'a'.repeat(33), security: 'open'})).toThrow('invalid_ssid');
    expect(() => validateWifi({ssid: 'LT', security: 'wpa-psk', psk: 'short'})).toThrow('invalid_psk');
    expect(() => validateWifi({ssid: 'LT', security: 'wpa-psk', psk: 'tab\tpassword'})).toThrow('invalid_psk');
    expect(() => validateWifi({ssid: 'LT 会場', security: 'wpa-psk', psk: 'password'})).not.toThrow();
  });
});

import {Reassembler} from './framing';
describe('reassembler', () => {
  it('round-trips out of order and ignores other ids', () => {
    const msg = new Uint8Array(1000).map((_, i) => i * 7);
    const frags = fragment(42, msg, 23).reverse();
    const r = new Reassembler(42);
    expect(r.push(Uint8Array.of(41, 0, 1, 9))).toBeNull();
    let out: Uint8Array | null = null;
    for (const f of frags) out = r.push(f) ?? out;
    expect(out).toEqual(msg);
  });
  it('rejects duplicates and inconsistent counts', () => {
    const r = new Reassembler(1);
    r.push(Uint8Array.of(1, 0, 2, 9));
    expect(r.push(Uint8Array.of(1, 0, 2, 9))).toBeNull(); // identical duplicate ignored
    expect(() => r.push(Uint8Array.of(1, 0, 2, 8))).toThrow('duplicate');
    expect(() => new Reassembler(1).push(Uint8Array.of(1, 2, 2, 9))).toThrow('malformed');
    const s = new Reassembler(1); s.push(Uint8Array.of(1, 0, 2, 9));
    expect(() => s.push(Uint8Array.of(1, 1, 3, 9))).toThrow('inconsistent');
  });
});
