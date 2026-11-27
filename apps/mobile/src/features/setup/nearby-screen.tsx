// Setup step 2: find the Stephania device over Bluetooth, claim it (with the QR
// secret) or open it as its owner.
import {router} from 'expo-router';
import {useEffect, useState} from 'react';
import {ActivityIndicator, View} from 'react-native';
import type {Device} from 'react-native-ble-plx';
import {ProvisioningSession} from '@stephania/provisioning-client';
import {Footnote, Icon, IconButton, List, ListIcon, Notice, Screen, Title, TopBar, useColor} from '@/shared/ui';
import {connectTransport, disconnect, ensurePermissions, scanDevices, waitPoweredOn} from './ble/manager';
import {describeSetupError} from './errors';
import {loadOwnerKey, ownerId, pendingSecret, saveOwnerKey, setActive, setPendingSecret} from './state';

type Phase = {kind: 'searching'} | {kind: 'connecting'; id: string; label: string} | {kind: 'error'; message: string};

/** Signal strength as bars plus a spoken word; dBm alone means little to most people. */
const signal = (rssi: number | null) => rssi == null ? {level: 0, label: 'Signal unknown'} : rssi > -60 ? {level: 3, label: 'Strong signal'}
  : rssi > -75 ? {level: 2, label: 'Good signal'} : {level: 1, label: 'Weak signal, move closer'};

export function NearbyScreen() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [phase, setPhase] = useState<Phase>({kind: 'searching'});
  const [scanRun, setScanRun] = useState(0);

  useEffect(() => {
    let stop: (() => void) | undefined;
    (async () => {
      if (!(await ensurePermissions())) return setPhase({kind: 'error', message: 'Bluetooth permission is required to find the Stephania device.'});
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
      setActive({deviceId: device.id, name: device.localName ?? device.name ?? 'Stephania', session});
      router.push('/setup/wifi');
      setPhase({kind: 'searching'});
    } catch (e) {
      await disconnect(device.id);
      setPhase({kind: 'error', message: describeSetupError(e)});
    }
  }

  const busy = phase.kind === 'connecting';
  const spinner = useColor('secondary');
  const rescan = () => { setDevices([]); setPhase({kind: 'searching'}); setScanRun(n => n + 1); };
  return (
    <Screen header={<TopBar onBack={() => router.back()} title="Nearby" trailing={<>
      {!pendingSecret() && <IconButton icon="qr" label="Set up a new Stephania device: scan its code" onPress={() => router.replace('/setup/scan')} />}
      <IconButton icon="refresh" label="Scan again" onPress={rescan} />
    </>} />} scroll>
      <Title>Nearby devices</Title>
      <List.Section footer={devices.length === 0 ? 'Plugged in and started? It takes about 30 seconds.' : undefined}>
        {devices.length === 0
          ? <List.Row title="Searching…" accessory={<ActivityIndicator color={spinner} />} />
          : devices.map(d => {
            const sig = signal(d.rssi);
            const connecting = busy && phase.id === d.id;
            return (
              <List.Row key={d.id} title={d.localName ?? d.name ?? 'Stephania'} detail={connecting ? phase.label : undefined}
                accessibilityLabel={`${d.localName ?? d.name ?? 'Stephania'}, ${connecting ? phase.label : sig.label}`}
                leading={<ListIcon icon={'bluetooth'} />}
                value={connecting ? undefined : <Icon name={(['signal0', 'signal1', 'signal2', 'signal3'] as const)[sig.level]!} size={18} color={sig.level >= 2 ? 'text' : 'warning'} />}
                accessory={connecting ? <ActivityIndicator color={spinner} /> : undefined}
                disabled={busy} onPress={() => void open(d)} />
            );
          })}
      </List.Section>
      {phase.kind === 'error' && <Notice tone="warning">{phase.message}</Notice>}
      <View className="flex-row items-center justify-center gap-2" accessible accessibilityLabel="Keep the phone within a few metres of the Stephania device">
        <Icon name="bluetooth" size={16} color="tertiary" /><Footnote tone="tertiary">Within a few metres</Footnote>
      </View>
    </Screen>
  );
}
