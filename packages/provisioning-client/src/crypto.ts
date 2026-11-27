// HMAC and sealing constructions from contracts/ble-protocol.md. Pure JS
// (@noble/hashes, @noble/ciphers) so it runs in React Native without native
// crypto modules.
import {chacha20poly1305} from '@noble/ciphers/chacha';
import {x25519} from '@noble/curves/ed25519';
import {hmac} from '@noble/hashes/hmac';
import {sha256} from '@noble/hashes/sha256';

const enc = new TextEncoder();

export const toHex = (b: Uint8Array) => Array.from(b, x => x.toString(16).padStart(2, '0')).join('');
export function fromHex(s: string): Uint8Array {
  if (s.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(s)) throw new Error('invalid hex');
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** u32-BE length prefixed concatenation, identical to the device. */
export function encodeParts(parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((n, p) => n + 4 + p.length, 0);
  const out = new Uint8Array(total);
  const view = new DataView(out.buffer);
  let o = 0;
  for (const p of parts) { view.setUint32(o, p.length); out.set(p, o + 4); o += 4 + p.length; }
  return out;
}

const mac = (key: Uint8Array, parts: (Uint8Array | string)[]) =>
  hmac(sha256, key, encodeParts(parts.map(p => (typeof p === 'string' ? enc.encode(p) : p))));

/**
 * Owner key (protocol v3): both sides derive it from the X25519 shared
 * secret, bound to the challenge nonce, the owner id and both public keys.
 */
export const deriveOwnerKey = (shared: Uint8Array, nonce: Uint8Array, ownerId: string, deviceKey: Uint8Array, phoneKey: Uint8Array) =>
  mac(shared, ['stephania-owner-key-v2', nonce, ownerId, deviceKey, phoneKey]);
/** The device's proof that it derived the same owner key. */
export const claimConfirmation = (ownerKey: Uint8Array, nonce: Uint8Array) => mac(ownerKey, ['stephania-claim-confirm-v1', nonce]);
/** X25519: public key of a 32-byte private key, and the shared secret. */
export const x25519PublicKey = (privateKey: Uint8Array) => x25519.getPublicKey(privateKey);
export const x25519Shared = (privateKey: Uint8Array, publicKey: Uint8Array) => x25519.getSharedSecret(privateKey, publicKey);
export const operationProof = (ownerKey: Uint8Array, nonce: Uint8Array, operation: string, payload: Uint8Array) =>
  mac(ownerKey, ['stephania-operation-v1', nonce, operation, payload]);

export const OWNER_ID = /^[A-Za-z0-9._-]{1,64}$/;

/**
 * Application-layer confidentiality: ChaCha20-Poly1305 with
 * key = MAC(owner_key, "stephania-seal-v1"), nonce = first 12 bytes of the
 * single-use operation nonce, AAD = operation name.
 */
export function seal(ownerKey: Uint8Array, opNonce: Uint8Array, operation: string, plaintext: Uint8Array): Uint8Array {
  return chacha20poly1305(mac(ownerKey, ['stephania-seal-v1']), opNonce.subarray(0, 12), new TextEncoder().encode(operation)).encrypt(plaintext);
}

/** Opens a value sealed by the device (e.g. management.enroll); null if forged. */
export function open(ownerKey: Uint8Array, opNonce: Uint8Array, operation: string, sealed: Uint8Array): Uint8Array | null {
  try {
    return chacha20poly1305(mac(ownerKey, ['stephania-seal-v1']), opNonce.subarray(0, 12), new TextEncoder().encode(operation)).decrypt(sealed);
  } catch {
    return null;
  }
}
