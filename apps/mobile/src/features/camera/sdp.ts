// Checks a native offer against what stephania-phone accepts (docs/PHONE.md):
// one video m-line per mapped track, H.264 constrained baseline (42e0xx) with
// packetization-mode=1, every mapped track id present in a=msid, and ICE
// candidates already gathered (the device answers non-trickle). A bad offer
// is a bug in the native sender; catching it here gives a clear error
// instead of a silent black source on the device.
import type {Offer} from '@modules/stephania-camera';

export class OfferError extends Error {}

type Section = {kind: string; lines: string[]};

function sections(sdp: string): Section[] {
  const out: Section[] = [];
  for (const raw of sdp.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith('m=')) out.push({kind: line.slice(2).split(' ')[0]!, lines: [line]});
    else out.at(-1)?.lines.push(line);
  }
  return out;
}

/** Constrained baseline at any level: profile_idc 0x42 with constraint_set1 (e.g. 42e01f, 42e034). */
export function isConstrainedBaseline(profileLevelId: string): boolean {
  if (!/^[0-9a-f]{6}$/i.test(profileLevelId) || profileLevelId.slice(0, 2) !== '42') return false;
  return (parseInt(profileLevelId.slice(2, 4), 16) & 0x40) !== 0;
}

/** Payload types whose fmtp is H.264 constrained baseline with packetization-mode=1. */
export function baselinePayloads(lines: string[]): string[] {
  const h264 = new Set(lines.flatMap(l => /^a=rtpmap:(\d+) H264\/90000$/i.exec(l)?.[1] ?? []));
  return lines.flatMap(l => {
    const m = /^a=fmtp:(\d+) (.*)$/.exec(l);
    if (!m || !h264.has(m[1]!)) return [];
    const params = Object.fromEntries(m[2]!.split(';').map(p => p.trim().split('=') as [string, string]));
    return params['profile-level-id'] && isConstrainedBaseline(params['profile-level-id']) && params['packetization-mode'] === '1' ? [m[1]!] : [];
  });
}

export function checkOffer(offer: Offer): void {
  const ids = Object.keys(offer.tracks);
  if (ids.length < 1 || ids.length > 2) throw new OfferError('one or two camera tracks expected');
  if (new Set(Object.values(offer.tracks)).size !== ids.length) throw new OfferError('each camera role may appear once');
  const video = sections(offer.sdp).filter(s => s.kind === 'video');
  if (video.length !== ids.length) throw new OfferError(`expected ${ids.length} video sections, got ${video.length}`);
  const seen = new Set<string>();
  for (const s of video) {
    const payloads = s.lines[0]!.split(' ').slice(3);
    const baseline = baselinePayloads(s.lines);
    if (!baseline.length || !payloads.includes(baseline[0]!)) throw new OfferError('video section without H.264 constrained baseline (42e0xx) packetization-mode=1');
    if (!s.lines.some(l => l === 'a=sendonly')) throw new OfferError('video sections must be sendonly');
    for (const l of s.lines) {
      const m = /^a=msid:\S+ (\S+)$/.exec(l);
      if (m) seen.add(m[1]!);
    }
  }
  for (const id of ids) if (!seen.has(id)) throw new OfferError(`track ${id} is not in the offer`);
  if (!/^a=candidate:/m.test(offer.sdp)) throw new OfferError('offer has no ICE candidates (gathering incomplete)');
}
