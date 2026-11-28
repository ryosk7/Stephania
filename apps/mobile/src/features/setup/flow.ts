// The setup steps that talk to the device, shared by the setup sheet and the
// Wi-Fi screen. contracts/ble-protocol.md v3: the first phone to claim an
// unclaimed device owns it.
import * as Crypto from 'expo-crypto';
import * as Haptics from 'expo-haptics';
import {ProvisioningError, ProvisioningSession} from '@stephania/provisioning-client';
import {saveEnrollment} from '@/shared/device';
import {connectTransport, disconnect} from './ble/manager';
import {activeSession, loadOwnerKey, loadPendingOwnerKey, ownerId, promoteOwnerKey, savePendingOwnerKey, setActive} from './state';

/**
 * Connect, become the owner (or open as the owner), and say whether the
 * device is already online (LAN cable or a Wi-Fi it already joined), in
 * which case Wi-Fi setup is skipped.
 *
 * A device this phone already owns that now reports UNCLAIMED is either the
 * same device after its card was rewritten or another device imitating it
 * (the report is not authenticated). Its owner key is only replaced when
 * the person says so (`replaceOwnerKey`); otherwise this throws
 * `owner_key_exists`.
 */
export async function openDevice(deviceId: string, name: string, {replaceOwnerKey = false} = {}): Promise<{online: boolean}> {
  const session = new ProvisioningSession(await connectTransport(deviceId));
  try {
    const info = await session.deviceInformation();
    if (info.ownership === 'UNCLAIMED') {
      if (!replaceOwnerKey && (await loadOwnerKey(deviceId))) throw new ProvisioningError('owner_key_exists');
      // Stored before the claim is sent: if the device keeps the claim but
      // the response is lost, the next attempt opens it with this key.
      const key = await session.claim(await ownerId(), Crypto.getRandomBytes(32), k => savePendingOwnerKey(deviceId, k));
      await promoteOwnerKey(deviceId, key);
      setActive({deviceId, name, session});
    } else {
      // A pending key comes from a claim whose answer was lost, so it is
      // newer than any confirmed key.
      const pending = await loadPendingOwnerKey(deviceId);
      const key = pending ?? (await loadOwnerKey(deviceId));
      if (!key) throw new ProvisioningError('not_owner');
      session.useOwnerKey(key);
      setActive({deviceId, name, session, pendingKey: pending ?? undefined});
    }
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
  // The sealed answer opened under the pending key: the device holds it.
  if (active.pendingKey) await promoteOwnerKey(active.deviceId, active.pendingKey);
  await saveEnrollment(active.deviceId, enrollment, active.name);
  void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
  await disconnect(active.deviceId);
  setActive(undefined);
  return active.deviceId;
}
