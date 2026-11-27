import type {IconName, Tone} from '@/shared/ui';
import type {Status} from './client';

/**
 * Broadcast state as shown everywhere (DESIGN.md §3.1). The engine's
 * `native_worker_active` means the encoder/transport worker runs, not that
 * YouTube confirmed ingest, so it reads "Streaming", never "Live on YouTube".
 */
export function broadcastState(s: Status | undefined): {tone: Tone; icon: IconName; label: string; pulse: boolean} {
  if (!s) return {tone: 'neutral', icon: 'broadcast', label: 'Unknown', pulse: false};
  switch (s.state) {
    case 'native_worker_active': return {tone: 'live', icon: 'broadcast', label: 'Streaming', pulse: true};
    case 'starting_or_retrying': return {tone: 'info', icon: 'refresh', label: s.last_error ? 'Retrying' : 'Starting', pulse: false};
    case 'stopping': return {tone: 'info', icon: 'stop', label: 'Stopping', pulse: false};
    default: return s.last_error
      ? {tone: 'warning', icon: 'warning', label: 'Stopped with an error', pulse: false}
      : {tone: 'neutral', icon: 'stop', label: 'Off air', pulse: false};
  }
}

/** Minutes and seconds since `since` (Unix ms), for the on-air chip: "12:04", "1:02:09". */
export function elapsedSince(since: number | null | undefined, now = Date.now()): string | undefined {
  if (since == null || !Number.isFinite(since) || now < since) return undefined;
  const t = since;
  const s = Math.floor((now - t) / 1000);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
}
