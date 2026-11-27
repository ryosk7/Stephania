// When the setup sheet shows (DESIGN.md §6). Used by Home through the app layer.
import {useCallback, useEffect, useRef, useState, type ReactNode} from 'react';
import {UUID} from '@stephania/provisioning-client';
import {accessorySetup} from '@modules/stephania-accessory-setup';
import {SetupSheet} from './setup-sheet';
import {useNearbyDevices, type NearbyDevice} from './use-nearby';

/**
 * When the setup sheet shows. iOS 18: Apple's AccessorySetupKit sheet
 * finds the device (no Bluetooth prompt), then ours runs setup at once.
 * Elsewhere ours scans itself: on first run (`firstRun`) it opens as soon
 * as a device answers; the plus button opens it any time.
 */
export function useSetupSheet(firstRun: boolean): {open(): void; sheet: ReactNode} {
  const [asked, setAsked] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const [picked, setPicked] = useState<NearbyDevice | null>(null);
  const system = accessorySetup.supported;
  const scanning = !system && (asked || (firstRun && !dismissed));
  const nearby = useNearbyDevices(scanning);

  const pick = useCallback(async () => {
    try {
      const a = await accessorySetup.pick(UUID.service, 'Stephania');
      if (a) setPicked(a);
    } catch { /* the system sheet shows its own errors */ }
  }, []);
  const offered = useRef(false);
  useEffect(() => {
    if (system && firstRun && !offered.current) { offered.current = true; void pick(); }
  }, [system, firstRun, pick]);

  const visible = system ? picked !== null : asked || (firstRun && !dismissed && nearby.devices.length > 0);
  return {
    open: () => (system ? void pick() : setAsked(true)),
    sheet: <SetupSheet visible={visible} devices={system ? (picked ? [picked] : []) : nearby.devices} autoStart={system}
      scanError={system ? '' : nearby.error} onClose={() => { setPicked(null); setAsked(false); setDismissed(true); }} />,
  };
}
