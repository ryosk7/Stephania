// LAN access to enrolled devices (management.enroll result: token, URLs,
// CA pin), kept in the platform secure store only. Shared: home, control,
// camera and pc-access all open a device from here.
import * as SecureStore from 'expo-secure-store';
import {toHex, type ManagementEnrollment} from '@t3box/provisioning-client';

const enrollmentKey = (deviceId: string) => 't3box.lan.' + toHex(new TextEncoder().encode(deviceId));
const INDEX_KEY = 't3box.lan.index';
const opts = {keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY};

export type EnrolledDevice = {deviceId: string; name: string};

export async function saveEnrollment(deviceId: string, e: ManagementEnrollment, name = 'T3Box') {
  await SecureStore.setItemAsync(enrollmentKey(deviceId), JSON.stringify(e), opts);
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

export async function loadEnrollment(deviceId: string): Promise<ManagementEnrollment | null> {
  const v = await SecureStore.getItemAsync(enrollmentKey(deviceId));
  return v ? (JSON.parse(v) as ManagementEnrollment) : null;
}

export async function forgetEnrollment(deviceId: string) {
  await SecureStore.deleteItemAsync(enrollmentKey(deviceId));
  const list = (await enrolledDevices()).filter(d => d.deviceId !== deviceId);
  await SecureStore.setItemAsync(INDEX_KEY, JSON.stringify(list), opts);
}
