// Polling and actions of the control screen, kept next to it (colocation).
import {useCallback, useEffect, useState} from 'react';
import {describeDeviceError, type AudioInput, type DeviceClient, type Status, type StoredScene} from '@/shared/device';

export const POLL_MS = 2000;

export function useControl(client: DeviceClient | undefined) {
  const [status, setStatus] = useState<Status>();
  const [scenes, setScenes] = useState<StoredScene[]>([]);
  const [audio, setAudio] = useState<AudioInput[]>([]);
  const [error, setError] = useState('');
  const [pending, setPending] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client) return;
    const [st, sc, au] = await Promise.all([client.status(), client.scenes(), client.audio()]);
    setStatus(st);
    setScenes([...sc].sort((a, b) => a.document.name.localeCompare(b.document.name)));
    setAudio(au.stored);
    setError('');
  }, [client]);

  useEffect(() => {
    if (!client) return;
    let alive = true;
    refresh().catch(e => alive && setError(describeDeviceError(e)));
    const t = setInterval(() => {
      client.status().then(st => { if (alive) { setStatus(st); setError(''); } }).catch(e => alive && setError(describeDeviceError(e)));
    }, POLL_MS);
    return () => { alive = false; clearInterval(t); };
  }, [client, refresh]);

  /** Runs one action at a time; `key` identifies it for per-control spinners. */
  const run = useCallback(async (key: string, f: () => Promise<unknown>) => {
    setPending(key);
    try { await f(); await refresh(); } catch (e) { setError(describeDeviceError(e)); } finally { setPending(null); }
  }, [refresh]);

  return {status, scenes, audio, error, pending, run};
}
