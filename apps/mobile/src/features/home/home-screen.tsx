// Home: one card per enrolled Stephania with its live state and the two things
// people do with it (DESIGN.md §6). Toolbar icons: set up, settings. A new
// device nearby opens the setup sheet by itself on first run.
import {router, useFocusEffect} from 'expo-router';
import {useCallback, useState, type ReactNode} from 'react';
import {View} from 'react-native';
import {broadcastState, enrolledDevices, useDeviceClient, useOnAirTime, type EnrolledDevice, type Status} from '@/shared/device';
import {ButtonRow, Card, Display, Footnote, Icon, IconButton, PrimaryButton, Screen, StatusChip} from '@/shared/ui';

/** The setup sheet and its trigger, composed in by the app layer (features/setup). */
export type UseSetup = (firstRun: boolean) => {open(): void; sheet: ReactNode};

export function HomeScreen({useSetup}: {useSetup: UseSetup}) {
  const [devices, setDevices] = useState<EnrolledDevice[] | null>(null);
  useFocusEffect(useCallback(() => { enrolledDevices().then(setDevices).catch(() => setDevices([])); }, []));
  const setup = useSetup(devices?.length === 0);

  return (
    <Screen scroll>
      <View className="mb-1 mt-2 flex-row items-center gap-2">
        <Display className="flex-1">Stephania</Display>
        <IconButton icon="plus" label="Set up a Stephania device" onPress={setup.open} />
        <IconButton icon="settings" label="Settings" onPress={() => router.push('/settings')} />
      </View>
      {devices?.length === 0 && <LookingCard />}
      {devices?.map(d => <DeviceCard key={d.deviceId} device={d} />)}
      {setup.sheet}
    </Screen>
  );
}

/** First run: the app is already looking; the sheet opens when a device answers. */
function LookingCard() {
  return (
    <Card>
      <View className="flex-row items-center gap-3" accessible accessibilityLabel="Looking for a Stephania device nearby">
        <Icon name="bluetooth" size={20} color="secondary" />
        <Footnote className="flex-1 text-[15px]">Looking for a Stephania device…</Footnote>
      </View>
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
