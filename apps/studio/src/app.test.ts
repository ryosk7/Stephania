import {describe, expect, it} from 'vitest';
import {parseRoute} from './app';

describe('routes', () => {
  it('parses hash routes and falls back to the dashboard', () => {
    expect(parseRoute('')).toBe('');
    expect(parseRoute('#/studio')).toBe('studio');
    expect(parseRoute('#/settings?x=1')).toBe('settings');
    expect(parseRoute('#audio')).toBe('audio');
    expect(parseRoute('#/nope')).toBe('');
  });
});
