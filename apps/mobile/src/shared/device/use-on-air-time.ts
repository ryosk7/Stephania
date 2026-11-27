import {useEffect, useState} from 'react';
import type {Status} from './client';
import {elapsedSince} from './status';

/** Elapsed on-air time, ticking every second while the T3Box reports a start time. */
export function useOnAirTime(status: Status | null | undefined): string | undefined {
  const since = status?.state === 'native_worker_active' ? status.streaming_since : undefined;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!since) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [since]);
  return elapsedSince(since, now);
}
