// Appearance preference, shared with Stephania Connect (DESIGN.md §4): follow
// the system by default, or a fixed light or dark theme. Pure (tested).
export const appearances = ['system', 'light', 'dark'] as const;
export type Appearance = (typeof appearances)[number];
export const APPEARANCE_KEY = 'stephania.appearance';

export function parseAppearance(stored: string | null | undefined): Appearance {
  return appearances.includes(stored as Appearance) ? (stored as Appearance) : 'system';
}

/** The theme to draw: the user's choice, or the system's when following it. */
export function resolveTheme(a: Appearance, systemDark: boolean): 'light' | 'dark' {
  return a === 'system' ? (systemDark ? 'dark' : 'light') : a;
}

export const appearanceLabel: Record<Appearance, string> = {system: 'Match the system', light: 'Light', dark: 'Dark'};
