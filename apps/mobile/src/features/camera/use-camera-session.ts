// Binds a CameraSession (platform-neutral policy, ./session.ts) to the
// screen: lifecycle, app background, preview of the selected cameras.
import {useEffect, useState} from 'react';
import {AppState} from 'react-native';
import NativeCamera, {type CameraMode, type Capabilities} from '@modules/stephania-camera';
import type {DeviceClient} from '@/shared/device';
import {CameraSession, type Snapshot} from './session';

export function useCameraSession(client: DeviceClient, mode: CameraMode) {
  const [session, setSession] = useState<CameraSession>();
  const [snap, setSnap] = useState<Snapshot>();
  const [caps, setCaps] = useState<Capabilities>();

  useEffect(() => {
    const s = new CameraSession(client, NativeCamera);
    const unsub = s.subscribe(setSnap);
    setSession(s);
    NativeCamera.capabilities().then(setCaps).catch(() => setCaps({roles: [], dualProfiles: [], hardwareH264: false}));
    return () => { unsub(); void s.release().finally(() => s.dispose()); };
  }, [client]);

  // iOS stops the cameras in the background; the session pauses and resumes.
  useEffect(() => {
    if (!session) return;
    const sub = AppState.addEventListener('change', st => void session.setBackground(st !== 'active'));
    return () => sub.remove();
  }, [session]);

  // Live preview of the selected cameras while not sending.
  useEffect(() => { if (session && caps) void session.preview(mode); }, [session, caps, mode]);

  return {session, snap, caps};
}

/** Seconds since `since` as m:ss / h:mm:ss, ticking once a second. */
export function useElapsed(since: number | null): string | undefined {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (since === null) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [since]);
  if (since === null) return undefined;
  const s = Math.max(0, Math.floor((now - since) / 1000));
  const hh = Math.floor(s / 3600), mm = Math.floor((s % 3600) / 60), ss = s % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  return hh ? `${hh}:${pad(mm)}:${pad(ss)}` : `${mm}:${pad(ss)}`;
}
