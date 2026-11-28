// Maintenance code state (docs/MAINTENANCE.md): locked until Face ID or the
// passcode, re-locked whenever the app leaves the foreground, computed on
// the device's clock when it could be measured over the LAN.
import * as LocalAuthentication from 'expo-local-authentication';
import {useCallback, useEffect, useRef, useState} from 'react';
import {AppState} from 'react-native';
import {otpSecretBytes} from '@stephania/provisioning-client';
import type {DeviceClient, DeviceClock} from '@/shared/device';
import {CLOCK_FRESH_MS, codeAt, type CodeView} from './code';

const AUTH_ERRORS: Record<string, string> = {
  passcode_not_set: 'Set a passcode on this phone to show maintenance codes.',
  not_enrolled: 'Set a passcode on this phone to show maintenance codes.',
  lockout: 'Too many attempts. Unlock the phone with its passcode, then try again.',
};

export function useMaintenanceCode(client: DeviceClient, otpSecret: string) {
  const [view, setView] = useState<CodeView>({state: 'locked'});
  const secret = useRef(otpSecretBytes(otpSecret));
  const clock = useRef<DeviceClock | null>(client.clock);
  const [now, setNow] = useState(Date.now());

  const lock = useCallback(() => setView({state: 'locked'}), []);

  const unlock = useCallback(async () => {
    setView({state: 'unlocking'});
    const r = await LocalAuthentication.authenticateAsync({promptMessage: 'Show the maintenance code', disableDeviceFallback: false});
    if (!r.success) {
      setView({state: 'locked', error: r.error === 'user_cancel' || r.error === 'system_cancel' || r.error === 'app_cancel' ? undefined : AUTH_ERRORS[r.error] ?? 'Could not confirm it is you.'});
      return;
    }
    // Measure the device clock first; unreachable devices fall back to the phone's.
    clock.current = await client.syncClock().catch(() => null) ?? client.clock;
    setNow(Date.now());
    setView({state: 'shown', ...codeAt(secret.current, Date.now(), clock.current)});
  }, [client]);

  // Tick while shown; re-measure the clock as it ages.
  const shown = view.state === 'shown';
  useEffect(() => {
    if (!shown) return;
    const tick = setInterval(() => setNow(Date.now()), 500);
    const sync = setInterval(() => { void client.syncClock().then(c => { clock.current = c ?? clock.current; }).catch(() => undefined); }, CLOCK_FRESH_MS / 2);
    return () => { clearInterval(tick); clearInterval(sync); };
  }, [shown, client]);

  // Leaving the app hides the code; coming back needs Face ID again.
  useEffect(() => {
    const sub = AppState.addEventListener('change', s => { if (s !== 'active') lock(); });
    return () => sub.remove();
  }, [lock]);

  const current: CodeView = shown ? {state: 'shown', ...codeAt(secret.current, now, clock.current)} : view;
  return {view: current, unlock, lock};
}
