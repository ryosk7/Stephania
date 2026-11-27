// Setup step 2: find the T3Box over Bluetooth, claim it (with the QR
// secret) or open it as its owner.
import {router} from 'expo-router';
import {useEffect, useState} from 'react';
import {ActivityIndicator, View} from 'react-native';
import type {Device} from 'react-native-ble-plx';
import {ProvisioningSession} from '@t3box/provisioning-client';
import {Body, Footnote, List, ListIcon, Notice, Screen, TextButton, Title, color, space, symbols} from '@/shared/ui';
import {connectTransport, disconnect, ensurePermissions, scanDevices, waitPoweredOn} from './ble/manager';
import {describeSetupError} from './errors';
import {loadOwnerKey, ownerId, pendingSecret, saveOwnerKey, setActive, setPendingSecret} from './state';

type Phase = {kind: 'searching'} | {kind: 'connecting'; id: string; label: string} | {kind: 'error'; message: string};

/** Signal strength in words; dBm alone means little to most people. */
const signal = (rssi: number | null) => rssi == null ? 'Signal unknown' : rssi > -60 ? 'Strong signal' : rssi > -75 ? 'Good signal' : 'Weak signal — move closer';

export function NearbyScreen() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [phase, setPhase] = useState<Phase>({kind: 'searching'});
  const [scanRun, setScanRun] = useState(0);

  useEffect(() => {
    let stop: (() => void) | undefined;
    (async () => {
      if (!(await ensurePermissions())) return setPhase({kind: 'error', message: 'Bluetooth permission is required to find the T3Box.'});
      try { await waitPoweredOn(); } catch (e) { return setPhase({kind: 'error', message: describeSetupError(e)}); }
      stop = scanDevices(
        d => setDevices(list => (list.some(x => x.id === d.id) ? list : [...list, d])),
        e => setPhase({kind: 'error', message: describeSetupError(e)}),
      );
    })();
    return () => stop?.();
  }, [scanRun]);

  async function open(device: Device) {
    setPhase({kind: 'connecting', id: device.id, label: 'Connecting…'});
    try {
      const session = new ProvisioningSession(await connectTransport(device.id));
      const info = await session.deviceInformation();
      const secret = pendingSecret();
      if (info.ownership === 'UNCLAIMED') {
        if (!secret) throw Object.assign(new Error('need_qr'), {code: 'need_qr'});
        setPhase({kind: 'connecting', id: device.id, label: 'Registering this phone as the owner…'});
        const key = await session.claim(secret, await ownerId());
        await saveOwnerKey(device.id, key);
        setPendingSecret(undefined);
      } else {
        const key = await loadOwnerKey(device.id);
        if (!key) throw Object.assign(new Error('not_owner'), {code: 'not_owner'});
        session.useOwnerKey(key);
      }
      setActive({deviceId: device.id, name: device.localName ?? device.name ?? 'T3Box', session});
      router.push('/setup/wifi');
      setPhase({kind: 'searching'});
    } catch (e) {
      await disconnect(device.id);
      setPhase({kind: 'error', message: describeSetupError(e)});
    }
  }

  const busy = phase.kind === 'connecting';
  return (
    <Screen scroll>
      <Title>Nearby T3Box</Title>
      <Footnote>Keep the phone within a few metres of the powered-on T3Box.</Footnote>
      <List.Section footer={devices.length === 0 ? 'Make sure the T3Box is plugged in and has finished starting (about 30 seconds).' : undefined}>
        {devices.length === 0
          ? <List.Row title="Searching…" accessory={<ActivityIndicator color={color.textSecondary} />} />
          : devices.map(d => (
            <List.Row key={d.id} title={d.localName ?? d.name ?? 'T3Box'} detail={busy && phase.id === d.id ? phase.label : signal(d.rssi)}
              leading={<ListIcon symbol={symbols.bluetooth} />}
              accessory={busy && phase.id === d.id ? <ActivityIndicator color={color.textSecondary} /> : undefined}
              disabled={busy} onPress={() => void open(d)} />
          ))}
      </List.Section>
      {phase.kind === 'error' && (
        <View style={{gap: space.s}}>
          <Notice tone="warning">{phase.message}</Notice>
          <TextButton title="Scan again" onPress={() => { setDevices([]); setPhase({kind: 'searching'}); setScanRun(n => n + 1); }} />
        </View>
      )}
      {!pendingSecret() && <Body tone="secondary">Setting up a new T3Box? <Body tone="info" onPress={() => router.replace('/setup/scan')}>Scan its setup code</Body> first.</Body>}
    </Screen>
  );
}
