// Maintenance codes (docs/MAINTENANCE.md): RFC 6238 over RFC 4226, HMAC-SHA-1,
// 6 digits, 30 s steps, as `stephania-maint` verifies them (src/totp.rs).
// SHA-1 is what the RFC and authenticator apps use; HMAC-SHA-1 is sound here.
import {hmac} from '@noble/hashes/hmac';
import {sha1} from '@noble/hashes/legacy';

export const TOTP_STEP_SECONDS = 30;
export const TOTP_DIGITS = 6;

/** RFC 4226 HOTP value for `counter`, reduced to `digits` digits. */
export function hotp(secret: Uint8Array, counter: number, digits = TOTP_DIGITS): number {
  const msg = new Uint8Array(8);
  // Counters stay far below 2^53; split into two 32-bit halves.
  new DataView(msg.buffer).setUint32(0, Math.floor(counter / 2 ** 32));
  new DataView(msg.buffer).setUint32(4, counter >>> 0);
  const mac = hmac(sha1, secret, msg);
  const o = mac[19]! & 0x0f;
  const value = ((mac[o]! & 0x7f) << 24) | (mac[o + 1]! << 16) | (mac[o + 2]! << 8) | mac[o + 3]!;
  return value % 10 ** digits;
}

/** The 6-digit code (zero-padded) for a Unix time in seconds. */
export function totpCode(secret: Uint8Array, unixSeconds: number): string {
  return String(hotp(secret, Math.floor(unixSeconds / TOTP_STEP_SECONDS))).padStart(TOTP_DIGITS, '0');
}

/** Seconds until the code for `unixSeconds` changes (1–30). */
export function totpSecondsLeft(unixSeconds: number): number {
  return TOTP_STEP_SECONDS - (Math.floor(unixSeconds) % TOTP_STEP_SECONDS);
}

/** Decodes the `otp_secret` delivered by management.enroll (base64). */
export function otpSecretBytes(base64: string): Uint8Array {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  const clean = base64.replace(/=+$/, '');
  const out: number[] = [];
  let bits = 0, acc = 0;
  for (const ch of clean) {
    const v = alphabet.indexOf(ch);
    if (v < 0) throw new Error('invalid base64');
    acc = (acc << 6) | v;
    bits += 6;
    if (bits >= 8) { bits -= 8; out.push((acc >> bits) & 0xff); }
  }
  return new Uint8Array(out);
}

/**
 * Short CA fingerprint, as `stephctl login` prints it: the first 16 bytes of
 * the SHA-256 as 8 groups of 4 upper-case hex digits.
 */
export function shortFingerprint(caSha256Hex: string): string {
  return (caSha256Hex.slice(0, 32).toUpperCase().match(/.{4}/g) ?? []).join(' ');
}
