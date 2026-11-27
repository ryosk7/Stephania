import {describe, expect, it} from 'vitest';
import {meterPercent} from './audio';

describe('meterPercent', () => {
  it('maps -60..0 dBFS to 0..100 % and clamps', () => {
    expect(meterPercent(-60)).toBe(0);
    expect(meterPercent(-30)).toBe(50);
    expect(meterPercent(0)).toBe(100);
    expect(meterPercent(-120)).toBe(0);
    expect(meterPercent(3)).toBe(100);
    expect(meterPercent(undefined)).toBe(0);
  });
});
