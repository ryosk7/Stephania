// The setup steps that talk to the device, shared by the setup sheet and the
// Wi-Fi screen. contracts/ble-protocol.md v3: the first phone to claim an
// unclaimed device owns it.
import * as Crypto from 'expo-crypto';
import * as Haptics from 'expo-haptics';
import {ProvisioningError, ProvisioningSession} from '@stephania/provisioning-client';
import {saveEnrollment} from '@/shared/device';
import {connectTransport, disconnect} from './ble/manager';
import {activeSession, loadOwnerKey, ownerId, saveOwnerKey, setActive} from './state';

/**
 * Connect, become the owner (or open as the owner), and say whether the
 * device is already online (LAN cable or a Wi-Fi it already joined), in
 * which case Wi-Fi setup is skipped.
 */
export async function openDevice(deviceId: string, name: string): Promise<{online: boolean}> {
  const session = new ProvisioningSession(await connectTransport(deviceId));
  try {
    const info = await session.deviceInformation();
    if (info.ownership === 'UNCLAIMED') {
      const key = await session.claim(await ownerId(), Crypto.getRandomBytes(32));
      await saveOwnerKey(deviceId, key);
    } else {
      const key = await loadOwnerKey(deviceId);
      if (!key) throw new ProvisioningError('not_owner');
      session.useOwnerKey(key);
    }
    setActive({deviceId, name, session});
    const status = await session.networkStatus();
    return {online: status.wired === true || status.state === 'CONNECTED'};
  } catch (e) {
    await disconnect(deviceId);
    throw e;
  }
}

/** Issue LAN access (sealed under the owner key) and remember the device. Returns its id. */
export async function finishSetup(): Promise<string> {
  const active = activeSession();
  if (!active) throw new ProvisioningError('disconnected');
  if (!(await active.session.managementAvailable())) throw new ProvisioningError('management_unavailable');
  const enrollment = await active.session.enrollManagement();
  await saveEnrollment(active.deviceId, enrollment, active.name);
  void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
  await disconnect(active.deviceId);
  setActive(undefined);
  return active.deviceId;
}
