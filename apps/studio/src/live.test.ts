import {describe, expect, it} from 'vitest';
import {initialLive, reduceLive} from './live';

describe('reduceLive', () => {
  it('reconciles from snapshot then follows events', () => {
    let s = reduceLive(initialLive, {event: 'snapshot', data: {engine: null, scenes: [{id: 'lt', revision: 3}]}});
    expect(s).toMatchObject({engine: 'unknown', scenes: {lt: 3}});
    s = reduceLive(s, {event: 'engine.connected', data: {state: 'idle', last_error: '', program: null}});
    expect(s).toMatchObject({engine: 'connected', streamState: 'idle', program: null});
    s = reduceLive(s, {event: 'stream.state_changed', data: {state: 'failed', last_error: 'rtmps refused'}});
    expect(s.lastError).toBe('rtmps refused');
    s = reduceLive(s, {event: 'program.changed', data: {scene_id: 'lt', revision: 3}});
    expect(s.program).toEqual({scene_id: 'lt', revision: 3});
    s = reduceLive(s, {event: 'engine.restarted', data: {state: 'idle', program: null}});
    expect(s).toMatchObject({engineRestarted: true, program: null});
    s = reduceLive(s, {event: 'scene.updated', data: {id: 'lt', revision: 4}});
    s = reduceLive(s, {event: 'scene.deleted', data: {id: 'lt'}});
    expect(s.scenes).toEqual({});
    s = reduceLive(s, {event: 'engine.unavailable', data: {}});
    expect(s.engine).toBe('unavailable');
  });

  it('ignores malformed and unknown events', () => {
    expect(reduceLive(initialLive, {event: 'scene.updated', data: {id: 1}})).toBe(initialLive);
    expect(reduceLive(initialLive, {event: 'future.event', data: {}})).toBe(initialLive);
  });
});
