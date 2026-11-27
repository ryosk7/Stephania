import {describe, expect, it} from 'vitest';
import {DEFAULT_APPEARANCE, parseAppearance} from './appearance';

describe('parseAppearance', () => {
  it('defaults to following the system', () => {
    expect(DEFAULT_APPEARANCE).toBe('system');
    expect(parseAppearance(null)).toBe('system');
    expect(parseAppearance(undefined)).toBe('system');
  });
  it('keeps a stored choice', () => {
    expect(parseAppearance('light')).toBe('light');
    expect(parseAppearance('dark')).toBe('dark');
    expect(parseAppearance('system')).toBe('system');
  });
  it('ignores values it does not know', () => {
    expect(parseAppearance('sepia')).toBe('system');
    expect(parseAppearance('')).toBe('system');
  });
});
