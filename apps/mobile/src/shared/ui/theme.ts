// Applies and remembers the appearance (light / dark / follow the system)
// through Uniwind, which switches every className between the light and dark
// tokens of src/global.css.
import AsyncStorage from '@react-native-async-storage/async-storage';
import {useSyncExternalStore} from 'react';
import {Uniwind, useCSSVariable} from 'uniwind';
import {APPEARANCE_KEY, DEFAULT_APPEARANCE, parseAppearance, type Appearance} from './appearance';

let current: Appearance = DEFAULT_APPEARANCE;
const listeners = new Set<() => void>();

function apply(a: Appearance) {
  current = a;
  Uniwind.setTheme(a);
  listeners.forEach(l => l());
}

/** Restores the saved choice; called once before the first screen renders. */
export async function restoreAppearance(): Promise<void> {
  let stored: string | null = null;
  try { stored = await AsyncStorage.getItem(APPEARANCE_KEY); } catch { /* first launch or storage unavailable: follow the system */ }
  apply(parseAppearance(stored));
}

export async function setAppearance(a: Appearance): Promise<void> {
  apply(a);
  await AsyncStorage.setItem(APPEARANCE_KEY, a);
}

/** The user's choice (not the resolved theme): 'system' while following the device. */
export function useAppearance(): Appearance {
  return useSyncExternalStore(l => { listeners.add(l); return () => { listeners.delete(l); }; }, () => current);
}

/** Semantic colour names from src/global.css, for props that take a colour rather than a className. */
export type ColorToken =
  | 'bg' | 'surface' | 'pressed' | 'fill' | 'border' | 'text' | 'secondary' | 'tertiary' | 'on-primary'
  | 'live' | 'warning' | 'on-warning' | 'success' | 'info' | 'track-off' | 'scrim' | 'mini-slides' | 'mini-camera' | 'mini-mark' | 'video' | 'white';

/** Resolved value of a colour token in the active theme (icons, spinners, animated colours). */
export function useColor(token: ColorToken): string {
  return String(useCSSVariable(`--color-${token}`) ?? '#808080');
}
