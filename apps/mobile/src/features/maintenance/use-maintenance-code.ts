// Maintenance code state (docs/MAINTENANCE.md): locked until Face ID or the
// passcode, re-locked whenever the app leaves the foreground, computed on
// the device's clock when it could be measured over the LAN. The secret is
// read from the secure store only on unlock and dropped on lock; when it is
// `protected` the OS itself asks for Face ID / Touch ID before releasing it.
import * as LocalAuthentication from 'expo-local-authentication';
import {useCallback, useEffect, useRef, useState} from 'react';
import {AppState} from 'react-native';
import {otpSecretBytes} from '@stephania/provisioning-client';
import {loadOtpSecret, OtpSecretError, type DeviceClient, type DeviceClock, type Enrollment} from '@/shared/device';
import {CLOCK_FRESH_MS, codeAt, type CodeView} from './code';

const AUTH_ERRORS: Record<string, string> = {
  passcode_not_set: 'Set a passcode on this phone to show maintenance codes.',
  not_enrolled: 'Set a passcode on this phone to show maintenance codes.',
  lockout: 'Too many attempts. Unlock the phone with its passcode, then try again.',
};

const PROMPT = 'Show the maintenance code';

export function useMaintenanceCode(client: DeviceClient, deviceId: string, maintenance: Exclude<Enrollment['maintenance'], 'none'>) {
  const [view, setView] = useState<CodeView>({state: 'locked'});
  const secret = useRef<Uint8Array | null>(null);
  const clock = useRef<DeviceClock | null>(client.clock);
  const [now, setNow] = useState(Date.now());

  const lock = useCallback(() => { secret.current = null; setView({state: 'locked'}); }, []);

  const unlock = useCallback(async () => {
    setView({state: 'unlocking'});
    if (maintenance === 'app-gated') {
      // No biometrics on this phone: the store cannot gate the secret, so ask here.
      const r = await LocalAuthentication.authenticateAsync({promptMessage: PROMPT, disableDeviceFallback: false});
      if (!r.success) {
        setView({state: 'locked', error: r.error === 'user_cancel' || r.error === 'system_cancel' || r.error === 'app_cancel' ? undefined : AUTH_ERRORS[r.error] ?? 'Could not confirm it is you.'});
        return;
      }
    }
    try {
      secret.current = otpSecretBytes(await loadOtpSecret(deviceId, PROMPT));
    } catch (e) {
      setView({state: 'locked', error: e instanceof OtpSecretError ? (e.reason === 'cancelled' ? undefined : e.message) : 'Could not read the maintenance code.'});
      return;
    }
    // Measure the device clock first; unreachable devices fall back to the phone's.
    clock.current = await client.syncClock().catch(() => null) ?? client.clock;
    if (!secret.current) return; // locked again (app left the foreground) meanwhile
    setNow(Date.now());
    setView({state: 'shown', ...codeAt(secret.current, Date.now(), clock.current)});
  }, [client, deviceId, maintenance]);

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

  const current: CodeView = shown && secret.current ? {state: 'shown', ...codeAt(secret.current, now, clock.current)} : shown ? {state: 'locked'} : view;
  return {view: current, unlock, lock};
}
