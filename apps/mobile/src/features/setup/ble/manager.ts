import {PermissionsAndroid, Platform} from 'react-native';
import {BleManager, State, type Device} from 'react-native-ble-plx';
import {UUID} from '@stephania/provisioning-client';
import {BlePlxTransport} from './transport';

let manager: BleManager | undefined;
export const ble = () => (manager ??= new BleManager());

export async function ensurePermissions(): Promise<boolean> {
  if (Platform.OS !== 'android') return true; // iOS prompts via Info.plist strings from the config plugin
  if (Number(Platform.Version) >= 31) {
    const r = await PermissionsAndroid.requestMultiple([
      PermissionsAndroid.PERMISSIONS.BLUETOOTH_SCAN,
      PermissionsAndroid.PERMISSIONS.BLUETOOTH_CONNECT,
    ]);
    return Object.values(r).every(v => v === PermissionsAndroid.RESULTS.GRANTED);
  }
  const r = await PermissionsAndroid.request(PermissionsAndroid.PERMISSIONS.ACCESS_FINE_LOCATION);
  return r === PermissionsAndroid.RESULTS.GRANTED;
}

export async function waitPoweredOn(timeoutMs = 5000): Promise<void> {
  if ((await ble().state()) === State.PoweredOn) return;
  await new Promise<void>((resolve, reject) => {
    const t = setTimeout(() => { sub.remove(); reject(new Error('bluetooth_off')); }, timeoutMs);
    const sub = ble().onStateChange(s => { if (s === State.PoweredOn) { clearTimeout(t); sub.remove(); resolve(); } }, true);
  });
}

/** Scan for devices advertising the Stephania provisioning service. */
export function scanDevices(onDevice: (d: Device) => void, onError: (e: Error) => void): () => void {
  void ble().startDeviceScan([UUID.service], null, (error, device) => {
    if (error) return onError(error);
    if (device) onDevice(device);
  });
  return () => { void ble().stopDeviceScan(); };
}

async function link(deviceId: string): Promise<number> {
  let device = await ble().connectToDevice(deviceId, {timeout: 15_000});
  if (Platform.OS === 'android') device = await ble().requestMTUForDevice(deviceId, 247);
  device = await ble().discoverAllServicesAndCharacteristicsForDevice(deviceId);
  // iOS negotiates the MTU itself; ble-plx reports it on the device.
  return Math.max(20, Math.min(device.mtu ?? 23, 512) - 3);
}

/**
 * Connect and return a v2 transport. No BLE pairing/bonding is requested:
 * the protocol authenticates and seals at the application layer, which
 * avoids the most common BlueZ/phone interoperability failures.
 */
export async function connectTransport(deviceId: string): Promise<BlePlxTransport> {
  await ble().stopDeviceScan();
  const att = await link(deviceId);
  return new BlePlxTransport(ble(), deviceId, att, async () => {
    try { await ble().cancelDeviceConnection(deviceId); } catch { /* already gone */ }
    await link(deviceId);
  });
}

export async function disconnect(deviceId: string) {
  try { await ble().cancelDeviceConnection(deviceId); } catch { /* already gone */ }
}
