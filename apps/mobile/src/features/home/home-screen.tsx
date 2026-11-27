// Home: one card per enrolled Stephania with its live state and the two things
// people do with it (DESIGN.md §6). Toolbar icons: nearby, set up, settings.
import {router, useFocusEffect} from 'expo-router';
import {useCallback, useState} from 'react';
import {View} from 'react-native';
import {broadcastState, enrolledDevices, useDeviceClient, useOnAirTime, type EnrolledDevice, type Status} from '@/shared/device';
import {ButtonRow, Card, Display, Footnote, Icon, IconButton, PrimaryButton, Screen, StatusChip} from '@/shared/ui';

export function HomeScreen() {
  const [devices, setDevices] = useState<EnrolledDevice[] | null>(null);
  useFocusEffect(useCallback(() => { enrolledDevices().then(setDevices).catch(() => setDevices([])); }, []));

  return (
    <Screen scroll>
      <View className="mb-1 mt-2 flex-row items-center gap-2">
        <Display className="flex-1">Stephania</Display>
        <IconButton icon="bluetooth" label="Connect a registered Stephania device nearby" onPress={() => router.push('/setup/nearby')} />
        <IconButton icon="plus" label="Set up a Stephania device" onPress={() => router.push('/setup/scan')} />
        <IconButton icon="settings" label="Settings" onPress={() => router.push('/settings')} />
      </View>
      {devices?.length === 0 && <SetupCard />}
      {devices?.map(d => <DeviceCard key={d.deviceId} device={d} />)}
    </Screen>
  );
}

/** First run: the three steps as pictures (PC → card → scan), one action. */
function SetupCard() {
  return (
    <Card>
      <Card.Header title="Set up a Stephania device" />
      <View className="flex-row items-center justify-center gap-3 py-3" accessible accessibilityLabel="Create a setup file on a PC, copy it to the microSD card, then scan the code shown on the PC">
        <Icon name="desktop" size={28} color="secondary" />
        <Icon name="arrow" size={18} color="tertiary" />
        <Icon name="qr" size={28} />
      </View>
      <PrimaryButton title="Scan" icon="qr" accessibilityLabel="Scan the setup code" onPress={() => router.push('/setup/scan')} />
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
  const onAir = useOnAirTime(status);

  const open = (path: '/device/[id]' | '/device/[id]/camera') => router.push({pathname: path, params: {id: device.deviceId}});
  const state = broadcastState(status ?? undefined);
  return (
    <Card>
      <Card.Header title={device.name}>
        {status === null
          ? <StatusChip tone="warning" icon="wifiOff" label="Not reachable" />
          : status !== undefined && <StatusChip tone={state.tone} icon={state.icon} label={state.label} value={onAir} pulse={state.pulse} />}
      </Card.Header>
      {status === null && (
        <View className="flex-row items-center gap-2">
          <Icon name="info" size={16} tone="warning" />
          <Footnote className="flex-1">Same Wi-Fi as this Stephania device?</Footnote>
        </View>
      )}
      <View className="mt-1">
        <ButtonRow>
          <IconButton shape="square" icon="camera" label="Send this phone's camera" onPress={() => open('/device/[id]/camera')} />
          <PrimaryButton title="Control" icon="control" onPress={() => open('/device/[id]')} />
        </ButtonRow>
      </View>
    </Card>
  );
}
