import {describe, expect, it} from 'vitest';
import {hotp, otpSecretBytes, shortFingerprint, totpCode, totpSecondsLeft} from './index';

const RFC_SECRET = new TextEncoder().encode('12345678901234567890');

describe('maintenance codes (RFC 6238 / 4226)', () => {
  it('matches the RFC 4226 HOTP vectors', () => {
    const expected = [755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489];
    expected.forEach((v, counter) => expect(hotp(RFC_SECRET, counter)).toBe(v));
  });

  it('matches the RFC 6238 SHA-1 vectors (8 and 6 digits)', () => {
    const vectors: [number, number][] = [[59, 94287082], [1111111109, 7081804], [1111111111, 14050471], [1234567890, 89005924],
      [2000000000, 69279037], [20000000000, 65353130]];
    for (const [t, eight] of vectors) {
      expect(hotp(RFC_SECRET, Math.floor(t / 30), 8)).toBe(eight);
      expect(totpCode(RFC_SECRET, t)).toBe(String(eight % 1_000_000).padStart(6, '0'));
    }
  });

  it('counts down within the step', () => {
    expect(totpSecondsLeft(60)).toBe(30);
    expect(totpSecondsLeft(89.9)).toBe(1);
  });

  it('decodes the enrolled secret', () => {
    const bytes = Uint8Array.from({length: 20}, (_, i) => i * 13);
    expect(otpSecretBytes(Buffer.from(bytes).toString('base64'))).toEqual(bytes);
    expect(() => otpSecretBytes('not*base64')).toThrow();
  });

  it('shortens the CA fingerprint to 8 groups', () => {
    expect(shortFingerprint('ab'.repeat(32))).toBe('ABAB ABAB ABAB ABAB ABAB ABAB ABAB ABAB');
  });
});
