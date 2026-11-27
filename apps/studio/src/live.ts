// Live device state folded from server events (src/api/events.ts). Pure so
// the reconciliation rules are unit-tested.
import type {ServerEvent} from './api/events';

export type Program = {scene_id: string; revision: number} | null;
export type LiveState = {
  engine: 'unknown' | 'connected' | 'unavailable';
  streamState?: string;
  lastError?: string;
  program: Program;
  /** Latest stored revision per scene id, as the server reports it. */
  scenes: Record<string, number>;
  /** Set when the engine instance changed since the page last saw it. */
  engineRestarted: boolean;
};

export const initialLive: LiveState = {engine: 'unknown', program: null, scenes: {}, engineRestarted: false};

type Status = {state?: string; last_error?: string; program?: Program};
const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});

function withStatus(s: LiveState, status: Status): LiveState {
  return {...s, engine: 'connected', streamState: status.state, lastError: status.last_error || undefined, program: status.program ?? null};
}

export function reduceLive(s: LiveState, e: ServerEvent): LiveState {
  const d = obj(e.data);
  switch (e.event) {
    case 'snapshot': {
      const scenes: Record<string, number> = {};
      for (const row of Array.isArray(d.scenes) ? d.scenes : []) {
        const r = obj(row);
        if (typeof r.id === 'string' && typeof r.revision === 'number') scenes[r.id] = r.revision;
      }
      const base = {...s, scenes};
      return d.engine ? withStatus(base, d.engine as Status) : {...base, engine: 'unknown'};
    }
    case 'engine.connected':
      return withStatus(s, d as Status);
    case 'engine.restarted':
      return {...withStatus(s, d as Status), engineRestarted: true};
    case 'engine.unavailable':
      return {...s, engine: 'unavailable'};
    case 'stream.state_changed':
      return {...s, streamState: typeof d.state === 'string' ? d.state : s.streamState, lastError: typeof d.last_error === 'string' && d.last_error ? d.last_error : undefined};
    case 'program.changed':
      return typeof d.scene_id === 'string' && typeof d.revision === 'number' ? {...s, program: {scene_id: d.scene_id, revision: d.revision}} : s;
    case 'scene.updated':
      return typeof d.id === 'string' && typeof d.revision === 'number' ? {...s, scenes: {...s.scenes, [d.id]: d.revision}} : s;
    case 'scene.deleted': {
      if (typeof d.id !== 'string') return s;
      const scenes = {...s.scenes};
      delete scenes[d.id];
      return {...s, scenes};
    }
    default:
      return s;
  }
}
