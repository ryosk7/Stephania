import {describe, expect, it} from 'vitest';
import {parseAppearance, resolveTheme} from './appearance';

describe('appearance', () => {
  it('follows the system unless the user picked a theme', () => {
    expect(parseAppearance(null)).toBe('system');
    expect(parseAppearance('dark')).toBe('dark');
    expect(parseAppearance('sepia')).toBe('system');
  });
  it('resolves the theme to draw', () => {
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('system', false)).toBe('light');
    expect(resolveTheme('light', true)).toBe('light');
    expect(resolveTheme('dark', false)).toBe('dark');
  });
});
