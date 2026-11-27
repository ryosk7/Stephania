import {describe, expect, it} from 'vitest';
import {DeviceClient, DeviceError, formatFingerprint, type NativePinned} from './device';

const PIN = 'ab'.repeat(32);
const enrollment = {token: 't3b_' + '0'.repeat(64), tokenId: 'x', urls: ['https://t3box.local:9443/', 'https://192.168.1.20:9443/'], caSha256: PIN};

function fake(plan: Record<string, (body: string | null) => {status: number; body: string} | Error>) {
  const calls: {url: string; method: string; headers: Record<string, string>; pin: string}[] = [];
  const native: NativePinned = {
    async request(url, method, headersJson, body, pin) {
      calls.push({url, method, headers: JSON.parse(headersJson), pin});
      const host = new URL(url).host;
      const r = plan[host]?.(body) ?? new Error('unreachable host');
      if (r instanceof Error) throw r;
      return {...r, headers: '{}'};
    },
  };
  return {native, calls};
}

describe('DeviceClient', () => {
  it('sends the enrolled token with the pin and falls back to the next address', async () => {
    const {native, calls} = fake({
      't3box.local:9443': () => new Error('mDNS name not resolvable'),
      '192.168.1.20:9443': () => ({status: 200, body: JSON.stringify({state: 'idle', program: null})}),
    });
    const c = new DeviceClient(enrollment, native);
    expect((await c.status()).state).toBe('idle');
    expect(calls.map(x => x.url)).toEqual(['https://t3box.local:9443/api/v1/status', 'https://192.168.1.20:9443/api/v1/status']);
    expect(calls.every(x => x.pin === PIN && x.headers.Authorization === 'Bearer ' + enrollment.token)).toBe(true);
    // The address that worked is tried first next time.
    await c.status();
    expect(calls.at(-1)!.url).toBe('https://192.168.1.20:9443/api/v1/status');
    expect(c.baseUrl).toBe('https://192.168.1.20:9443/');
  });

  it('maps the error envelope and does not retry HTTP errors on other addresses', async () => {
    const {native, calls} = fake({'t3box.local:9443': () => ({status: 409, body: JSON.stringify({error: {code: 'program_inactive', message: 'no stream'}})})});
    const e = await new DeviceClient(enrollment, native).transition('lt', 3).catch(x => x);
    expect(e).toBeInstanceOf(DeviceError);
    expect([e.code, e.status]).toEqual(['program_inactive', 409]);
    expect(calls).toHaveLength(1);
  });

  it('sends JSON bodies (fade transition, audio mute)', async () => {
    const bodies: (string | null)[] = [];
    const {native} = fake({'t3box.local:9443': b => { bodies.push(b); return {status: 200, body: '{}'}; }});
    const c = new DeviceClient(enrollment, native);
    await c.transition('lt', 3, 500);
    await c.setMuted('mic', true);
    expect(bodies.map(b => JSON.parse(b!))).toEqual([
      {scene_id: 'lt', revision: 3, transition: {type: 'fade', duration_ms: 500}},
      {inputs: [{id: 'mic', muted: true}]},
    ]);
  });

  it('fails closed when every address fails and refuses non-https enrollments', async () => {
    const {native} = fake({});
    await expect(new DeviceClient(enrollment, native).status()).rejects.toMatchObject({code: 'unreachable'});
    expect(() => new DeviceClient({...enrollment, urls: ['http://t3box.local/']}, native)).toThrow(DeviceError);
    expect(() => new DeviceClient({...enrollment, caSha256: 'nope'}, native)).toThrow(DeviceError);
  });

  it('formats fingerprints for comparison', () => {
    expect(formatFingerprint('ab12cd34ef')).toBe('AB12 CD34 EF');
  });
});
