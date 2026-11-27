import {describe, expect, it} from 'vitest';
import {broadcastState, elapsedSince} from './status';

const base = {streaming_requested: true, last_error: '', program: null};

describe('broadcastState', () => {
  it('maps engine states to a tone, an icon and a spoken label', () => {
    expect(broadcastState({...base, state: 'native_worker_active'})).toEqual({tone: 'live', icon: 'broadcast', label: 'Streaming', pulse: true});
    expect(broadcastState({...base, state: 'starting_or_retrying', last_error: 'x'}).label).toBe('Retrying');
    expect(broadcastState({...base, state: 'idle', streaming_requested: false}).icon).toBe('stop');
    expect(broadcastState({...base, state: 'idle', last_error: 'boom'}).tone).toBe('warning');
    expect(broadcastState(undefined).label).toBe('Unknown');
  });
});

describe('elapsedSince', () => {
  const t0 = Date.parse('2026-11-27T10:00:00Z');
  it('formats minutes and seconds, and hours when needed', () => {
    expect(elapsedSince(t0, t0 + 724_000)).toBe('12:04');
    expect(elapsedSince(t0, t0 + 3_729_000)).toBe('1:02:09');
    expect(elapsedSince(t0, t0 + 5_000)).toBe('0:05');
  });
  it('shows nothing without a start in the past', () => {
    expect(elapsedSince(undefined, t0)).toBeUndefined();
    expect(elapsedSince(null, t0)).toBeUndefined();
    expect(elapsedSince(t0 + 10_000, t0)).toBeUndefined();
  });
});
