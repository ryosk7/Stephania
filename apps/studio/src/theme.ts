// Applies the appearance to <html class="dark"> and keeps it in step with
// the system setting while following it. Stored per browser.
import {useSyncExternalStore} from 'react';
import {APPEARANCE_KEY, parseAppearance, resolveTheme, type Appearance} from './appearance';

const media = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : undefined;
const listeners = new Set<() => void>();
let current: Appearance = 'system';

function read(): Appearance {
  try { return parseAppearance(localStorage.getItem(APPEARANCE_KEY)); } catch { return 'system'; }
}

function apply() {
  document.documentElement.classList.toggle('dark', resolveTheme(current, media?.matches ?? false) === 'dark');
  listeners.forEach(l => l());
}

/** Called once before the first render, so the page never flashes the wrong theme. */
export function initAppearance() {
  current = read();
  media?.addEventListener('change', apply);
  apply();
}

export function setAppearance(a: Appearance) {
  current = a;
  try { localStorage.setItem(APPEARANCE_KEY, a); } catch { /* private mode: applies for this page only */ }
  apply();
}

export function useAppearance(): Appearance {
  return useSyncExternalStore(l => { listeners.add(l); return () => { listeners.delete(l); }; }, () => current);
}
