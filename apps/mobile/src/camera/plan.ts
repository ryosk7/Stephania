// Encoding plans for the phone camera tracks (docs/MASTER-ARCHITECTURE.md
// §12, docs/PHONE.md "Thermal adaptation"). Pure policy: the native sender
// applies whatever plan it is given, so the same table drives every platform.
//
// §12 profiles, best first: rear/front 720p30+720p30, 720p30+540p24,
// 540p24+540p24, single camera. The supplementary (front, audience) camera
// degrades first; the presenter (rear) camera stays on air longest. A
// phone only gets the dual profiles its hardware reported it can run
// concurrently (Capabilities.dualProfiles, from AVCaptureMultiCamSession
// hardware cost); a thermal step never moves *up* past that ceiling.
import type {CameraMode, Capabilities, DualProfile, ThermalState, TrackPlan} from '../../modules/t3box-camera';

type Shape = Omit<TrackPlan, 'role' | 'active'>;

export const P720_30: Shape = {width: 1280, height: 720, fps: 30, maxBitrateKbps: 2500};
export const P540_24: Shape = {width: 960, height: 540, fps: 24, maxBitrateKbps: 1200};
const P540_24_LOW: Shape = {...P540_24, maxBitrateKbps: 900};

/** §12 dual profiles, best first. */
export const DUAL_PROFILES: DualProfile[] = ['720p30+720p30', '720p30+540p24', '540p24+540p24'];
const DUAL_SHAPES: Record<DualProfile, [rear: Shape, front: Shape]> = {
  '720p30+720p30': [P720_30, P720_30],
  '720p30+540p24': [P720_30, P540_24],
  '540p24+540p24': [P540_24, P540_24],
};

// How many steps down the profile list each thermal state forces.
const DUAL_STEP: Record<ThermalState, number> = {nominal: 0, fair: 1, serious: 2, critical: 3};
// Single camera: 720p30 until `serious`, then 540p24, lower bitrate at `critical`.
const SINGLE: Record<ThermalState, Shape> = {nominal: P720_30, fair: P720_30, serious: P540_24, critical: P540_24_LOW};

export function rolesFor(mode: CameraMode): ('rear' | 'front')[] {
  return mode === 'dual' ? ['rear', 'front'] : [mode];
}

/** Best dual profile the phone supports, or null if it cannot run both cameras at once. */
export function bestDual(caps: Capabilities): DualProfile | null {
  return DUAL_PROFILES.find(p => caps.dualProfiles.includes(p)) ?? null;
}

export function planFor(mode: CameraMode, thermal: ThermalState, caps: Capabilities): TrackPlan[] {
  if (mode !== 'dual') return [{role: mode, active: true, ...SINGLE[thermal]}];
  const supported = DUAL_PROFILES.filter(p => caps.dualProfiles.includes(p));
  if (!supported.length) throw new Error('this phone cannot run both cameras at once');
  const step = DUAL_STEP[thermal];
  if (step >= supported.length) {
    // Below the last dual profile: single camera (rear stays, front off). The
    // front track stays negotiated but inactive so it can come back without
    // renegotiation when the phone cools down.
    const [rear, front] = DUAL_SHAPES[supported.at(-1)!];
    return [{role: 'rear', active: true, ...rear}, {role: 'front', active: false, ...front}];
  }
  const [rear, front] = DUAL_SHAPES[supported[step]!];
  return [{role: 'rear', active: true, ...rear}, {role: 'front', active: true, ...front}];
}

/** Capture formats are chosen once per session for the best plan; later plans only scale down. */
export const capturePlan = (mode: CameraMode, caps: Capabilities) => planFor(mode, 'nominal', caps);
