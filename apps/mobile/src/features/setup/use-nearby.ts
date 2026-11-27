// Stephania devices advertising nearby that this phone has not set up yet,
// in the order they were found (so the sheet does not reshuffle while the
// person swipes). Scans only while `enabled`.
import {useEffect, useState} from 'react';
import {enrolledDevices} from '@/shared/device';
import {ensurePermissions, scanDevices, waitPoweredOn} from './ble/manager';
import {describeSetupError} from './errors';

export type NearbyDevice = {id: string; name: string};

export function useNearbyDevices(enabled: boolean): {devices: NearbyDevice[]; error: string} {
  const [devices, setDevices] = useState<NearbyDevice[]>([]);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!enabled) return;
    let stop: (() => void) | undefined;
    let alive = true;
    setDevices([]); setError('');
    (async () => {
      if (!(await ensurePermissions())) return alive && setError('Bluetooth permission is needed to find the Stephania device.');
      try { await waitPoweredOn(); } catch (e) { return alive && setError(describeSetupError(e)); }
      const known = new Set((await enrolledDevices()).map(d => d.deviceId));
      if (!alive) return;
      stop = scanDevices(
        d => {
          if (known.has(d.id)) return;
          const name = d.localName ?? d.name ?? 'Stephania';
          setDevices(list => (list.some(x => x.id === d.id) ? list : [...list, {id: d.id, name}]));
        },
        e => setError(describeSetupError(e)),
      );
    })();
    return () => { alive = false; stop?.(); };
  }, [enabled]);

  return {devices, error};
}
