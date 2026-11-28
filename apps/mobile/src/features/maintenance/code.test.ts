import {describe, expect, it} from 'vitest';
import {totpCode} from '@stephania/provisioning-client';
import {CLOCK_FRESH_MS, codeAt} from './code';

const secret = new TextEncoder().encode('12345678901234567890');
const NOW = 1_800_000_000_000;

describe('maintenance code view', () => {
  it('uses the device clock while the measured offset is fresh', () => {
    const clock = {offsetMs: 120_000, atMs: NOW - 1000};
    const v = codeAt(secret, NOW, clock);
    expect(v).toMatchObject({source: 'device', code: totpCode(secret, (NOW + 120_000) / 1000)});
    expect(v.code).not.toBe(totpCode(secret, NOW / 1000));
  });

  it('falls back to the phone clock when the offset is stale or missing', () => {
    expect(codeAt(secret, NOW, null)).toMatchObject({source: 'phone', code: totpCode(secret, NOW / 1000)});
    expect(codeAt(secret, NOW, {offsetMs: 120_000, atMs: NOW - CLOCK_FRESH_MS})).toMatchObject({source: 'phone'});
    // A phone clock that jumped backwards does not trust the old offset either.
    expect(codeAt(secret, NOW, {offsetMs: 120_000, atMs: NOW + 60_000})).toMatchObject({source: 'phone'});
  });

  it('counts down to the next step', () => {
    expect(codeAt(secret, 1_800_000_010_000, null).secondsLeft).toBe(20);
  });
});
