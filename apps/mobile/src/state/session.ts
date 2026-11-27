// App-wide provisioning state. The setup secret lives only in memory until
// the claim succeeds; afterwards only the derived owner key is kept, in the
// platform secure store (Keychain / Keystore), never in plain storage.
import * as Crypto from 'expo-crypto';
import * as SecureStore from 'expo-secure-store';
import {fromHex, toHex, type ManagementEnrollment, type ProvisioningSession} from '@t3box/provisioning-client';

let setupSecret: Uint8Array | undefined;
let active: {deviceId: string; name: string; session: ProvisioningSession} | undefined;

export const setPendingSecret = (s: Uint8Array | undefined) => { setupSecret = s; };
export const pendingSecret = () => setupSecret;
export const setActive = (a: typeof active) => { active = a; };
export const activeSession = () => active;

// SecureStore keys allow [A-Za-z0-9._-]; Android device ids contain ':'.
const keyFor = (deviceId: string) => 't3box.owner.' + toHex(new TextEncoder().encode(deviceId));

export async function saveOwnerKey(deviceId: string, key: Uint8Array) {
  await SecureStore.setItemAsync(keyFor(deviceId), toHex(key), {keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY});
}

export async function loadOwnerKey(deviceId: string): Promise<Uint8Array | null> {
  const v = await SecureStore.getItemAsync(keyFor(deviceId));
  return v ? fromHex(v) : null;
}

/** Stable per-install owner id (shown to the device; not secret). */
export async function ownerId(): Promise<string> {
  let id = await SecureStore.getItemAsync('t3box.owner_id');
  if (!id) {
    id = 'phone-' + toHex(Crypto.getRandomBytes(6));
    await SecureStore.setItemAsync('t3box.owner_id', id);
  }
  return id;
}

const enrollmentKey = (deviceId: string) => 't3box.lan.' + toHex(new TextEncoder().encode(deviceId));

export type EnrolledDevice = {deviceId: string; name: string};
const INDEX_KEY = 't3box.lan.index';

/** LAN access token + CA pin from management.enroll (secure store only). */
export async function saveEnrollment(deviceId: string, e: ManagementEnrollment, name = 'T3Box') {
  await SecureStore.setItemAsync(enrollmentKey(deviceId), JSON.stringify(e), {keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY});
  const list = (await enrolledDevices()).filter(d => d.deviceId !== deviceId);
  await SecureStore.setItemAsync(INDEX_KEY, JSON.stringify([...list, {deviceId, name}]), {keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY});
}

/** Devices this phone holds LAN access for (no secrets in the list). */
export async function enrolledDevices(): Promise<EnrolledDevice[]> {
  const v = await SecureStore.getItemAsync(INDEX_KEY);
  try { return v ? (JSON.parse(v) as EnrolledDevice[]) : []; } catch { return []; }
}

export async function forgetEnrollment(deviceId: string) {
  await SecureStore.deleteItemAsync(enrollmentKey(deviceId));
  const list = (await enrolledDevices()).filter(d => d.deviceId !== deviceId);
  await SecureStore.setItemAsync(INDEX_KEY, JSON.stringify(list), {keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY});
}

export async function loadEnrollment(deviceId: string): Promise<ManagementEnrollment | null> {
  const v = await SecureStore.getItemAsync(enrollmentKey(deviceId));
  return v ? (JSON.parse(v) as ManagementEnrollment) : null;
}
