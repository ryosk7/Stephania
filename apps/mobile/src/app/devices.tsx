import {router} from 'expo-router';
import {useEffect, useState} from 'react';
import {FlatList, Pressable, Text} from 'react-native';
import type {Device} from 'react-native-ble-plx';
import {ProvisioningSession} from '@t3box/provisioning-client';
import {connectTransport, disconnect, ensurePermissions, scanDevices, waitPoweredOn} from '../ble/manager';
import {loadOwnerKey, ownerId, pendingSecret, saveOwnerKey, setActive, setPendingSecret} from '../state/session';
import {describeError, Screen, s} from '../ui';

export default function Devices() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [status, setStatus] = useState('Looking for nearby T3Box devices…');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let stop: (() => void) | undefined;
    (async () => {
      if (!(await ensurePermissions())) return setStatus('Bluetooth permission is required.');
      try { await waitPoweredOn(); } catch (e) { return setStatus(describeError(e)); }
      stop = scanDevices(
        d => setDevices(list => (list.some(x => x.id === d.id) ? list : [...list, d])),
        e => setStatus(describeError(e)),
      );
    })();
    return () => stop?.();
  }, []);

  async function open(device: Device) {
    setBusy(true);
    setStatus('Connecting…');
    try {
      const session = new ProvisioningSession(await connectTransport(device.id));
      const info = await session.deviceInformation();
      const secret = pendingSecret();
      if (info.ownership === 'UNCLAIMED') {
        if (!secret) throw Object.assign(new Error('need_qr'), {code: 'need_qr'});
        setStatus('Registering as the owner…');
        const key = await session.claim(secret, await ownerId());
        await saveOwnerKey(device.id, key);
        setPendingSecret(undefined);
      } else {
        const key = await loadOwnerKey(device.id);
        if (!key) throw Object.assign(new Error('not_owner'), {code: 'not_owner'});
        session.useOwnerKey(key);
      }
      setActive({deviceId: device.id, name: device.localName ?? device.name ?? 'T3Box', session});
      router.push('/wifi');
    } catch (e) {
      await disconnect(device.id);
      setStatus((e as {code?: string}).code === 'need_qr' ? 'This T3Box has no owner yet. Scan its setup QR code first.' : describeError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Screen>
      <Text style={s.p}>{status}</Text>
      <FlatList
        data={devices}
        keyExtractor={d => d.id}
        renderItem={({item}) => (
          <Pressable disabled={busy} style={s.row} onPress={() => void open(item)} accessibilityRole="button">
            <Text style={s.h}>{item.localName ?? item.name ?? 'T3Box'}</Text>
            <Text style={s.p}>Signal {item.rssi ?? '?'} dBm</Text>
          </Pressable>
        )}
      />
    </Screen>
  );
}
