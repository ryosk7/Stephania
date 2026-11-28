// Provisioning state of the setup flow. Only the owner key derived during
// the claim is kept, in the platform secure store (Keychain / Keystore).
// A key is first stored as *pending* (before the claim is sent) and becomes
// the device's owner key once the device has proved it holds the same key,
// so a lost claim response never leaves the phone without it.
import * as Crypto from 'expo-crypto';
import * as SecureStore from 'expo-secure-store';
import {fromHex, toHex, type ProvisioningSession} from '@stephania/provisioning-client';

/**
 * `pendingKey`: the session uses a key whose claim was never confirmed; it
 * is promoted once an operation sealed under it succeeds. `identityConfirmed`:
 * the person confirmed this is their device before a Wi-Fi password is sent.
 */
export type ActiveSetup = {deviceId: string; name: string; session: ProvisioningSession; pendingKey?: Uint8Array; identityConfirmed?: boolean};
let active: ActiveSetup | undefined;

export const setActive = (a: ActiveSetup | undefined) => { active = a; };
export const activeSession = () => active;

// SecureStore keys allow [A-Za-z0-9._-]; Android device ids contain ':'.
const keyFor = (deviceId: string) => 'stephania.owner.' + toHex(new TextEncoder().encode(deviceId));
const pendingKeyFor = (deviceId: string) => 'stephania.owner-pending.' + toHex(new TextEncoder().encode(deviceId));
const opts = {keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY};

export async function saveOwnerKey(deviceId: string, key: Uint8Array) {
  await SecureStore.setItemAsync(keyFor(deviceId), toHex(key), opts);
}

export async function loadOwnerKey(deviceId: string): Promise<Uint8Array | null> {
  const v = await SecureStore.getItemAsync(keyFor(deviceId));
  return v ? fromHex(v) : null;
}

/** A key derived for a claim that has been sent but not confirmed yet. */
export async function savePendingOwnerKey(deviceId: string, key: Uint8Array) {
  await SecureStore.setItemAsync(pendingKeyFor(deviceId), toHex(key), opts);
}

export async function loadPendingOwnerKey(deviceId: string): Promise<Uint8Array | null> {
  const v = await SecureStore.getItemAsync(pendingKeyFor(deviceId));
  return v ? fromHex(v) : null;
}

/** The device proved it holds `key`: it becomes the owner key. */
export async function promoteOwnerKey(deviceId: string, key: Uint8Array) {
  await saveOwnerKey(deviceId, key);
  await SecureStore.deleteItemAsync(pendingKeyFor(deviceId));
}

/** Stable per-install owner id (shown to the device; not secret). */
export async function ownerId(): Promise<string> {
  let id = await SecureStore.getItemAsync('stephania.owner_id');
  if (!id) {
    id = 'phone-' + toHex(Crypto.getRandomBytes(6));
    await SecureStore.setItemAsync('stephania.owner_id', id);
  }
  return id;
}
