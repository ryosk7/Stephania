import {useEffect, useState} from 'react';
import PinnedFetch from '@modules/stephania-pinned-fetch';
import {DeviceClient} from './client';
import {enrolledDevices, loadEnrollment} from './enrollment';

export type DeviceHandle =
  | {state: 'loading'}
  | {state: 'missing'}
  | {state: 'ready'; client: DeviceClient; name: string; host: string};

/** Opens the enrolled device `deviceId` (secure store → pinned client). */
export function useDeviceClient(deviceId: string | undefined): DeviceHandle {
  const [handle, setHandle] = useState<DeviceHandle>({state: 'loading'});
  useEffect(() => {
    let alive = true;
    if (!deviceId) { setHandle({state: 'missing'}); return; }
    Promise.all([loadEnrollment(deviceId), enrolledDevices()])
      .then(([e, list]) => {
        if (!alive) return;
        if (!e) { setHandle({state: 'missing'}); return; }
        const host = (() => { try { return new URL(e.urls[0]!).hostname; } catch { return ''; } })();
        setHandle({state: 'ready', client: new DeviceClient(e, PinnedFetch), name: list.find(d => d.deviceId === deviceId)?.name ?? 'Stephania', host});
      })
      .catch(() => alive && setHandle({state: 'missing'}));
    return () => { alive = false; };
  }, [deviceId]);
  return handle;
}
