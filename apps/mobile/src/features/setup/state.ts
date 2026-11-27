// Provisioning state of the setup flow. Only the owner key derived during
// the claim is kept, in the platform secure store (Keychain / Keystore).
import * as Crypto from 'expo-crypto';
import * as SecureStore from 'expo-secure-store';
import {fromHex, toHex, type ProvisioningSession} from '@stephania/provisioning-client';

export type ActiveSetup = {deviceId: string; name: string; session: ProvisioningSession};
let active: ActiveSetup | undefined;

export const setActive = (a: ActiveSetup | undefined) => { active = a; };
export const activeSession = () => active;

// SecureStore keys allow [A-Za-z0-9._-]; Android device ids contain ':'.
const keyFor = (deviceId: string) => 'stephania.owner.' + toHex(new TextEncoder().encode(deviceId));
const opts = {keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY};

export async function saveOwnerKey(deviceId: string, key: Uint8Array) {
  await SecureStore.setItemAsync(keyFor(deviceId), toHex(key), opts);
}

export async function loadOwnerKey(deviceId: string): Promise<Uint8Array | null> {
  const v = await SecureStore.getItemAsync(keyFor(deviceId));
  return v ? fromHex(v) : null;
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
