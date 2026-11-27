// Home: one card per enrolled T3Box with its live state and the two things
// people do with it; empty state is a single "set up" card (DESIGN.md §6).
import {router, useFocusEffect} from 'expo-router';
import {useCallback, useState} from 'react';
import {View} from 'react-native';
import {broadcastState, enrolledDevices, useDeviceClient, type EnrolledDevice, type Status} from '@/shared/device';
import {ButtonRow, Card, Footnote, LargeTitle, PrimaryButton, Screen, SecondaryButton, StatusPill, TextButton, space, symbols} from '@/shared/ui';

export function HomeScreen() {
  const [devices, setDevices] = useState<EnrolledDevice[] | null>(null);
  useFocusEffect(useCallback(() => { enrolledDevices().then(setDevices).catch(() => setDevices([])); }, []));

  return (
    <Screen scroll footer={<TextButton title="Connect to a registered T3Box nearby" onPress={() => router.push('/setup/nearby')} />}>
      <LargeTitle>T3Box</LargeTitle>
      {devices?.length === 0 && <SetupCard />}
      {devices?.map(d => <DeviceCard key={d.deviceId} device={d} />)}
      {devices && devices.length > 0 && <TextButton title="Set up another T3Box" onPress={() => router.push('/setup/scan')} />}
    </Screen>
  );
}

function SetupCard() {
  return (
    <Card>
      <Card.Header title="Set up a T3Box" />
      <Card.Body>
        <Footnote>1. On a PC, open the setup page and create a secret.</Footnote>
        <Footnote>2. Copy t3box-setup.txt to the microSD card, insert it and power the T3Box on.</Footnote>
        <Footnote>3. Scan the QR code shown on the PC.</Footnote>
      </Card.Body>
      <PrimaryButton title="Scan setup code" symbol={symbols.qr} onPress={() => router.push('/setup/scan')} />
    </Card>
  );
}

function DeviceCard({device}: {device: EnrolledDevice}) {
  const handle = useDeviceClient(device.deviceId);
  const [status, setStatus] = useState<Status | null | undefined>();
  useFocusEffect(useCallback(() => {
    if (handle.state !== 'ready') return;
    let alive = true;
    handle.client.status().then(s => alive && setStatus(s)).catch(() => alive && setStatus(null));
    return () => { alive = false; };
  }, [handle]));

  const open = (path: '/device/[id]' | '/device/[id]/camera') => router.push({pathname: path, params: {id: device.deviceId}});
  const state = status === null ? {tone: 'warning' as const, label: 'Not reachable', pulse: false} : broadcastState(status ?? undefined);
  return (
    <Card>
      <Card.Header title={device.name}>
        {status !== undefined && <StatusPill tone={state.tone} label={state.label} pulse={state.pulse} />}
      </Card.Header>
      {status === null && <Footnote>Check that the phone is on the same Wi-Fi as the T3Box.</Footnote>}
      <View style={{marginTop: space.xs}}>
        <ButtonRow>
          <SecondaryButton title="Camera" symbol={symbols.camera} onPress={() => open('/device/[id]/camera')} />
          <PrimaryButton title="Control" onPress={() => open('/device/[id]')} />
        </ButtonRow>
      </View>
    </Card>
  );
}
