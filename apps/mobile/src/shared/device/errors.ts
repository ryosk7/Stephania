import {DeviceError} from './client';

/** User-facing text for management API errors (shared by every device screen). */
export function describeDeviceError(e: unknown): string {
  if (e instanceof DeviceError) {
    const m: Record<string, string> = {
      unreachable: 'Cannot reach the T3Box. Check that the phone is on the same Wi-Fi (connections with a different certificate are refused).',
      unauthorized: 'This phone\'s access is no longer valid. Issue it again from Wi-Fi setup.',
      program_inactive: 'Not streaming, so the scene cannot be switched.',
      engine_unavailable: 'The media engine is not responding.',
    };
    return m[e.code] ?? `${e.code}: ${e.message}`;
  }
  return e instanceof Error ? e.message : String(e);
}
