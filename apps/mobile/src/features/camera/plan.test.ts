import {describe, expect, it} from 'vitest';
import type {Capabilities, ThermalState} from '@modules/t3box-camera';
import {bestDual, planFor} from './plan';

const ALL: Capabilities = {roles: ['rear', 'front'], dualProfiles: ['720p30+720p30', '720p30+540p24', '540p24+540p24'], hardwareH264: true};
const LOW: Capabilities = {...ALL, dualProfiles: ['540p24+540p24']};
const SINGLE_ONLY: Capabilities = {...ALL, dualProfiles: []};
const brief = (caps: Capabilities, t: ThermalState) =>
  planFor('dual', t, caps).map(p => (p.active ? `${p.role}:${p.height}p${p.fps}` : `${p.role}:off`)).join(' ');

describe('planFor', () => {
  it('walks the §12 dual profiles down with heat, front camera first', () => {
    expect(brief(ALL, 'nominal')).toBe('rear:720p30 front:720p30');
    expect(brief(ALL, 'fair')).toBe('rear:720p30 front:540p24');
    expect(brief(ALL, 'serious')).toBe('rear:540p24 front:540p24');
    expect(brief(ALL, 'critical')).toBe('rear:540p24 front:off');
  });

  it('never exceeds what the phone reported it can capture concurrently', () => {
    expect(brief(LOW, 'nominal')).toBe('rear:540p24 front:540p24');
    expect(brief(LOW, 'fair')).toBe('rear:540p24 front:off');
    expect(bestDual(LOW)).toBe('540p24+540p24');
    expect(bestDual(SINGLE_ONLY)).toBeNull();
    expect(() => planFor('dual', 'nominal', SINGLE_ONLY)).toThrow(/both cameras/);
  });

  it('keeps one camera at 720p30 until the phone is seriously hot', () => {
    const h = (t: ThermalState) => planFor('rear', t, SINGLE_ONLY)[0]!;
    expect([h('nominal').height, h('fair').height, h('serious').height, h('critical').height]).toEqual([720, 720, 540, 540]);
    expect(h('critical').maxBitrateKbps).toBeLessThan(h('serious').maxBitrateKbps);
    expect(planFor('front', 'nominal', SINGLE_ONLY)).toEqual([expect.objectContaining({role: 'front', active: true})]);
  });

  it('every plan has positive, bounded encoder settings', () => {
    for (const t of ['nominal', 'fair', 'serious', 'critical'] as const)
      for (const p of [...planFor('dual', t, ALL), ...planFor('rear', t, ALL)]) {
        expect(p.width * 9).toBe(p.height * 16);
        expect(p.fps).toBeGreaterThan(0);
        expect(p.maxBitrateKbps).toBeLessThanOrEqual(2500);
      }
  });
});
