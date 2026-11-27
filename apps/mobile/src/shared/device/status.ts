import type {Tone} from '@/shared/ui';
import type {Status} from './client';

/**
 * Broadcast state as shown everywhere (DESIGN.md §3.1). The engine's
 * `native_worker_active` means the encoder/transport worker runs, not that
 * YouTube confirmed ingest, so it reads "Streaming", never "Live on YouTube".
 */
export function broadcastState(s: Status | undefined): {tone: Tone; label: string; pulse: boolean} {
  if (!s) return {tone: 'neutral', label: 'Unknown', pulse: false};
  switch (s.state) {
    case 'native_worker_active': return {tone: 'live', label: 'Streaming', pulse: true};
    case 'starting_or_retrying': return {tone: 'info', label: s.last_error ? 'Retrying' : 'Starting', pulse: false};
    case 'stopping': return {tone: 'info', label: 'Stopping', pulse: false};
    default: return s.last_error ? {tone: 'warning', label: 'Stopped with an error', pulse: false} : {tone: 'neutral', label: 'Off air', pulse: false};
  }
}
