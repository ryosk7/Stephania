// Pure part of the maintenance code view (tested in code.test.ts).
import {totpCode, totpSecondsLeft} from '@stephania/provisioning-client';
import type {DeviceClock} from '@/shared/device';
import type {ClockSource} from './code-card';

/** An offset older than this is measured again before it is trusted. */
export const CLOCK_FRESH_MS = 5 * 60_000;

export type CodeView =
  | {state: 'locked'; error?: string}
  | {state: 'unlocking'}
  | {state: 'shown'; code: string; secondsLeft: number; source: ClockSource};

/** The code and countdown at phone time `nowMs`, on the device clock if fresh. */
export function codeAt(secret: Uint8Array, nowMs: number, clock: DeviceClock | null): {code: string; secondsLeft: number; source: ClockSource} {
  const fresh = clock !== null && nowMs - clock.atMs < CLOCK_FRESH_MS && nowMs >= clock.atMs;
  const t = (nowMs + (fresh ? clock.offsetMs : 0)) / 1000;
  return {code: totpCode(secret, t), secondsLeft: totpSecondsLeft(t), source: fresh ? 'device' : 'phone'};
}

