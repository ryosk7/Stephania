// Appearance preference (DESIGN.md §4): follow the system by default, or a
// fixed light or dark theme chosen in Settings. Pure, so it is unit-tested.
export const appearances = ['system', 'light', 'dark'] as const;
export type Appearance = (typeof appearances)[number];

export const DEFAULT_APPEARANCE: Appearance = 'system';
export const APPEARANCE_KEY = 't3box.appearance';

/** A stored value, or the default when it is missing or unknown (e.g. written by a newer app). */
export function parseAppearance(stored: string | null | undefined): Appearance {
  return appearances.includes(stored as Appearance) ? (stored as Appearance) : DEFAULT_APPEARANCE;
}

export const appearanceLabel: Record<Appearance, string> = {system: 'Match the system', light: 'Light', dark: 'Dark'};
