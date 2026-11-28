// LAN access to enrolled devices (management.enroll result: token, URLs,
// CA pin), kept in the platform secure store only. Shared: home, control,
// camera and pc-access all open a device from here.
//
// The maintenance code secret (docs/MAINTENANCE.md) is not part of that
// item: it has its own item stored with `requireAuthentication`, so the OS
// releases it only after Face ID / Touch ID (iOS keychain access control,
// Android Keystore user authentication), not merely because the phone is
// unlocked. The enrollment item only records that it exists
// (`otpProtected`). On a phone without biometrics the secret stays in the
// enrollment item behind the in-app Face ID or passcode prompt, and moves
// to its own item once biometrics are available.
import * as SecureStore from 'expo-secure-store';
import {Platform} from 'react-native';
import {toHex, type ManagementEnrollment} from '@stephania/provisioning-client';

const enrollmentKey = (deviceId: string) => 'stephania.lan.' + toHex(new TextEncoder().encode(deviceId));
const otpKey = (deviceId: string) => enrollmentKey(deviceId) + '.otp';
const INDEX_KEY = 'stephania.lan.index';
const opts = {keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY};
const otpOpts = (authenticationPrompt: string): SecureStore.SecureStoreOptions =>
  ({...opts, requireAuthentication: true, authenticationPrompt});

/** What the enrollment item holds: `otpSecret` only on phones without biometrics. */
type StoredEnrollment = ManagementEnrollment & {otpProtected?: boolean};

/** An enrolled device as screens see it: never carries the code secret. */
export type Enrollment = Omit<ManagementEnrollment, 'otpSecret'> & {
  /** Where the maintenance code secret is: OS-gated item, in-app gate, or none. */
  maintenance: 'protected' | 'app-gated' | 'none';
};

/** The maintenance code secret could not be read (cancelled, failed, or invalidated). */
export class OtpSecretError extends Error {
  constructor(readonly reason: 'cancelled' | 'failed' | 'invalidated', message: string) {
    super(message);
  }
}

export type EnrolledDevice = {deviceId: string; name: string};

export async function saveEnrollment(deviceId: string, e: ManagementEnrollment, name = 'Stephania') {
  const {otpSecret, ...rest} = e;
  // A new enrollment rotates the secret: drop any older protected copy.
  await SecureStore.deleteItemAsync(otpKey(deviceId));
  const stored: StoredEnrollment = otpSecret === undefined ? rest
    : await protectOtpSecret(deviceId, otpSecret) ? {...rest, otpProtected: true} : {...rest, otpSecret};
  await SecureStore.setItemAsync(enrollmentKey(deviceId), JSON.stringify(stored), opts);
  const list = (await enrolledDevices()).filter(d => d.deviceId !== deviceId);
  await SecureStore.setItemAsync(INDEX_KEY, JSON.stringify([...list, {deviceId, name}]), opts);
}

/**
 * Renames a device on this phone only (the label in the app). The device's
 * own host name, certificate and mDNS name do not change, so nothing has to
 * be set up again.
 */
export async function renameDevice(deviceId: string, name: string) {
  const trimmed = name.trim().slice(0, 40);
  if (!trimmed) throw new Error('A name is required.');
  const list = await enrolledDevices();
  await SecureStore.setItemAsync(INDEX_KEY, JSON.stringify(list.map(d => (d.deviceId === deviceId ? {...d, name: trimmed} : d))), opts);
  return trimmed;
}

/** Devices this phone holds LAN access for (no secrets in the list). */
export async function enrolledDevices(): Promise<EnrolledDevice[]> {
  const v = await SecureStore.getItemAsync(INDEX_KEY);
  try { return v ? (JSON.parse(v) as EnrolledDevice[]) : []; } catch { return []; }
}

async function loadStored(deviceId: string): Promise<StoredEnrollment | null> {
  const v = await SecureStore.getItemAsync(enrollmentKey(deviceId));
  return v ? (JSON.parse(v) as StoredEnrollment) : null;
}

/**
 * Writes the code secret to its own authentication-gated item. False when
 * the phone has no biometrics enrolled or the store refuses (then the
 * caller keeps the in-app gate). On Android the write itself asks for a
 * fingerprint, so it is only attempted when the user is already at a
 * prompt (setup, Show code); iOS writes without a prompt.
 */
async function protectOtpSecret(deviceId: string, secret: string): Promise<boolean> {
  try {
    if (!SecureStore.canUseBiometricAuthentication()) return false;
    await SecureStore.setItemAsync(otpKey(deviceId), secret, otpOpts('Protect the maintenance code'));
    return true;
  } catch {
    return false;
  }
}

/** Moves a secret kept in the enrollment item (older app versions) to its own item. */
async function migrateOtpSecret(deviceId: string, stored: StoredEnrollment): Promise<StoredEnrollment> {
  if (stored.otpSecret === undefined || !(await protectOtpSecret(deviceId, stored.otpSecret))) return stored;
  const {otpSecret: _moved, ...rest} = stored;
  const next: StoredEnrollment = {...rest, otpProtected: true};
  await SecureStore.setItemAsync(enrollmentKey(deviceId), JSON.stringify(next), opts);
  return next;
}

function publicEnrollment(stored: StoredEnrollment): Enrollment {
  const {otpSecret, otpProtected, ...rest} = stored;
  return {...rest, maintenance: otpProtected ? 'protected' : otpSecret !== undefined ? 'app-gated' : 'none'};
}

/** The enrolled device's LAN access, without the maintenance code secret. */
export async function loadEnrollment(deviceId: string): Promise<Enrollment | null> {
  let stored = await loadStored(deviceId);
  if (!stored) return null;
  // iOS writes the gated item without a prompt, so older items move as
  // soon as they are opened; Android waits for the next Show code.
  if (Platform.OS === 'ios') stored = await migrateOtpSecret(deviceId, stored).catch(() => stored!);
  return publicEnrollment(stored);
}

/**
 * Reads the maintenance code secret. A `protected` secret is released by
 * the OS after Face ID / Touch ID; an `app-gated` one is returned as is, so
 * the caller must have confirmed the user first (and it is moved to a
 * protected item on the way when possible). Throws OtpSecretError.
 */
export async function loadOtpSecret(deviceId: string, prompt: string): Promise<string> {
  const stored = await loadStored(deviceId);
  if (stored?.otpProtected) {
    let secret: string | null;
    try {
      secret = await SecureStore.getItemAsync(otpKey(deviceId), otpOpts(prompt));
    } catch (e) {
      const cancelled = /cancel/i.test(e instanceof Error ? e.message : String(e));
      throw new OtpSecretError(cancelled ? 'cancelled' : 'failed', cancelled ? 'Cancelled.' : 'Could not confirm it is you.');
    }
    // Keys are invalidated when the enrolled faces or fingerprints change.
    if (secret === null) throw new OtpSecretError('invalidated', 'Face ID or Touch ID changed on this phone, so its maintenance codes were locked. Set the device up again over Bluetooth to get them back.');
    return secret;
  }
  if (stored?.otpSecret === undefined) throw new OtpSecretError('failed', 'This phone has no maintenance codes for the device.');
  await migrateOtpSecret(deviceId, stored).catch(() => undefined);
  return stored.otpSecret;
}

export async function forgetEnrollment(deviceId: string) {
  await SecureStore.deleteItemAsync(otpKey(deviceId));
  await SecureStore.deleteItemAsync(enrollmentKey(deviceId));
  const list = (await enrolledDevices()).filter(d => d.deviceId !== deviceId);
  await SecureStore.setItemAsync(INDEX_KEY, JSON.stringify(list), opts);
}
