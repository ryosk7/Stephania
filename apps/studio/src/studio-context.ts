// What every page of the signed-in Studio needs, provided by App above the router.
import {createContext, use} from 'react';
import type {ConnectionState} from './api/events';
import type {createClient, Me} from './api/client';
import type {LiveState} from './live';

export type StudioContextValue = {
  api: ReturnType<typeof createClient>;
  me: Me;
  live: LiveState;
  conn: ConnectionState | 'off';
  canEdit: boolean;
  onMessage(m: string): void;
  logout(): void;
};

export const StudioContext = createContext<StudioContextValue | null>(null);

export function useStudio(): StudioContextValue {
  const v = use(StudioContext);
  if (!v) throw new Error('useStudio outside StudioContext');
  return v;
}
