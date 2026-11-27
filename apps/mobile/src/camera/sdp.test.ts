import {readFileSync} from 'node:fs';
import {describe, expect, it} from 'vitest';
import {baselinePayloads, checkOffer, OfferError} from './sdp';

// Shape of a libwebrtc unified-plan offer after codec preferences were set
// to H.264 42e01f only (two sendonly transceivers, gathered host candidate).
const SDP = readFileSync(new URL('./fixtures/offer-dual.sdp', import.meta.url), 'utf8');
const TRACKS = {'rear-1': 'rear', 'front-1': 'front'} as const;

describe('checkOffer', () => {
  it('accepts a gathered dual H.264 baseline offer', () => {
    expect(() => checkOffer({sdp: SDP, tracks: TRACKS})).not.toThrow();
    expect(baselinePayloads(SDP.split('\n'))).toEqual(['102', '102']);
  });

  it('accepts CRLF line endings', () => {
    expect(() => checkOffer({sdp: SDP.replace(/\n/g, '\r\n'), tracks: TRACKS})).not.toThrow();
  });

  const bad: [string, string, Record<string, 'rear' | 'front'>, RegExp][] = [
    ['high profile only', SDP.replaceAll('profile-level-id=42e01f', 'profile-level-id=640c1f'), TRACKS, /42e01f/],
    ['packetization-mode 0', SDP.replaceAll('packetization-mode=1', 'packetization-mode=0'), TRACKS, /42e01f/],
    ['VP8 instead of H.264', SDP.replaceAll('H264/90000', 'VP8/90000'), TRACKS, /H.264/],
    ['not gathered', SDP.replace(/^a=candidate:.*\n/m, ''), TRACKS, /candidates/],
    ['sendrecv', SDP.replaceAll('a=sendonly', 'a=sendrecv'), TRACKS, /sendonly/],
    ['unknown track id', SDP, {'rear-1': 'rear', 'other': 'front'}, /other/],
    ['mapping for one track, two sections', SDP, {'rear-1': 'rear'}, /expected 1 video/],
    ['same role twice', SDP, {'rear-1': 'rear', 'front-1': 'rear'}, /once/],
    ['no tracks', SDP, {}, /one or two/],
  ];
  for (const [name, sdp, tracks, re] of bad)
    it(`rejects ${name}`, () => {
      const e = (() => { try { checkOffer({sdp, tracks}); } catch (x) { return x; } })();
      expect(e).toBeInstanceOf(OfferError);
      expect((e as Error).message).toMatch(re);
    });
});
