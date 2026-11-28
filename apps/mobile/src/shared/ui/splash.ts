// Whether the launch splash has gone. The root layout hides it; screens that
// open a sheet by themselves on launch wait for this, so the sheet does not
// appear over the splash.
import {useSyncExternalStore} from 'react';

let gone = false;
const listeners = new Set<() => void>();

/** Called by the root layout once the splash has faded out. */
export function markSplashGone(): void {
  if (gone) return;
  gone = true;
  for (const l of listeners) l();
}

export function useSplashGone(): boolean {
  return useSyncExternalStore(l => { listeners.add(l); return () => { listeners.delete(l); }; }, () => gone);
}
