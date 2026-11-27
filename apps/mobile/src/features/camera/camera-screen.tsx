// Send this phone's camera(s) to the Stephania device (docs/PHONE.md; DESIGN.md §6
// Camera). Renders CameraSession state only; capture, encoding and
// transport are native.
import * as Haptics from 'expo-haptics';
import {router} from 'expo-router';
import {useEffect, useState} from 'react';
import {StyleSheet, View} from 'react-native';
import {CameraPreview, type CameraMode, type CameraRole} from '@modules/stephania-camera';
import {DeviceGate, useDeviceClient, type DeviceClient} from '@/shared/device';
import {ButtonRow, DestructiveButton, Footnote, Icon, IconButton, LiveBadge, Mono, Notice, PrimaryButton, Screen, SecondaryButton, Segmented, StatusChip, type IconName, type Tone} from '@/shared/ui';
import {bestDual} from './plan';
import type {Snapshot} from './session';
import {useCameraSession, useElapsed} from './use-camera-session';

const STATUS: Record<Snapshot['state'], {label: string; tone: Tone; icon: IconName}> = {
  idle: {label: 'Ready', tone: 'neutral', icon: 'camera'},
  starting: {label: 'Opening camera', tone: 'info', icon: 'camera'},
  connecting: {label: 'Connecting', tone: 'info', icon: 'refresh'},
  live: {label: 'Live', tone: 'live', icon: 'broadcast'},
  reconnecting: {label: 'Reconnecting', tone: 'warning', icon: 'refresh'},
  busy: {label: 'Another phone is connected', tone: 'warning', icon: 'warning'},
  paused: {label: 'Paused', tone: 'neutral', icon: 'stop'},
  error: {label: 'Stopped', tone: 'neutral', icon: 'stop'},
};

const THERMAL: Record<Snapshot['thermal'], string | null> = {
  nominal: null,
  fair: 'Warm · quality slightly reduced',
  serious: 'Hot · quality reduced',
  critical: 'Very hot · quality at minimum',
};

const ROLE: Record<CameraRole, {label: string; icon: IconName}> = {
  rear: {label: 'Rear camera', icon: 'camera'},
  front: {label: 'Front camera', icon: 'selfie'},
};

export function CameraScreen({deviceId}: {deviceId: string}) {
  const handle = useDeviceClient(deviceId);
  return <DeviceGate handle={handle}>{handle.state === 'ready' && <Camera client={handle.client} />}</DeviceGate>;
}

function Camera({client}: {client: DeviceClient}) {
  const [mode, setMode] = useState<CameraMode>('rear');
  const {session, snap, caps} = useCameraSession(client, mode);
  const state = snap?.state ?? 'idle';
  const sending = !['idle', 'busy', 'error'].includes(state);
  const shownMode = sending ? snap!.mode : mode;
  const roles: CameraRole[] = shownMode === 'dual' ? ['rear', 'front'] : [shownMode];
  const dual = caps ? bestDual(caps) : null;

  // Elapsed time since the session went live.
  const [liveSince, setLiveSince] = useState<number | null>(null);
  useEffect(() => {
    if (state === 'live') {
      setLiveSince(t => {
        if (t === null) void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
        return t ?? Date.now();
      });
    } else if (!sending) setLiveSince(null);
  }, [state, sending]);
  const elapsed = useElapsed(liveSince);

  const status = STATUS[state];
  const thermal = snap ? THERMAL[snap.thermal] : null;
  const message = snap?.message
    || (state === 'live' && snap?.degraded ? 'Front camera paused to protect the phone. It comes back when the phone cools down.' : '');

  return (
    <Screen footer={<Footer state={state} sending={sending} onStart={() => void session?.start(mode)} onStop={() => void session?.stop()}
      onTakeOver={() => { void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning); void session?.takeOver(); }}
      onCancel={() => void session?.stop()} ready={!!session} />}>
      <View className="min-h-11 flex-row items-center justify-between">
        <IconButton icon={'close'} label="Close" onPress={() => router.back()} />
        {state === 'live' ? <LiveBadge elapsed={elapsed} />
          : <StatusChip tone={status.tone} icon={status.icon} label={status.label} value={state === 'reconnecting' && snap ? `${snap.attempt}` : undefined} />}
        <View className="w-11" />
      </View>
      {thermal && <Notice tone="warning" icon={'thermometer'}>{thermal}</Notice>}

      <View className="flex-1 justify-center gap-3">
        {roles.map((role, i) => (
          <View key={role} className={`aspect-video items-center justify-center overflow-hidden rounded-card bg-video ${roles.length === 2 && i === 1 ? 'w-[62%] self-end' : 'w-full'}`}>
            {caps?.roles.includes(role)
              ? <CameraPreview camera={role} mirrored={role === 'front'} style={StyleSheet.absoluteFill} />
              : <Icon name={ROLE[role].icon} size={36} color="tertiary" />}
            <View className="absolute bottom-2.5 left-2.5 h-[30px] flex-row items-center gap-1.5 rounded-[9px] bg-black/60 px-[7px]"
              accessible accessibilityLabel={`${ROLE[role].label}${sending && snap?.degraded && role === 'front' ? ', paused' : ''}`}>
              <Icon name={ROLE[role].icon} size={17} color="white" />
              {sending && snap?.degraded && role === 'front' && <Icon name="stop" size={13} color="warning" />}
            </View>
          </View>
        ))}
      </View>

      <View className="gap-2">
        <Segmented label="Cameras" value={shownMode} onChange={setMode} locked={sending}>
          <Segmented.Item value="rear" label="Rear camera" icon="camera" unavailable={caps ? !caps.roles.includes('rear') : true} />
          <Segmented.Item value="front" label="Front camera" icon="selfie" unavailable={caps ? !caps.roles.includes('front') : true} />
          <Segmented.Item value="dual" label="Both cameras" icon="bothCameras" unavailable={!dual} />
        </Segmented>
        {caps && !dual
          ? <Footnote className="text-center">One camera at a time on this phone</Footnote>
          : <Mono tone="secondary" className="text-center" accessibilityLabel={dual ? `Both cameras up to ${dual}` : undefined}>{dual ? dual.replace('+', ' + ') : ' '}</Mono>}
        {message !== '' && <Notice tone={state === 'error' ? 'warning' : 'neutral'}>{message}</Notice>}
      </View>
    </Screen>
  );
}

/** Bottom action: Go live / Stop sending, or the take-over choice when busy. */
function Footer({state, sending, ready, onStart, onStop, onTakeOver, onCancel}: {
  state: Snapshot['state']; sending: boolean; ready: boolean;
  onStart(): void; onStop(): void; onTakeOver(): void; onCancel(): void;
}) {
  if (state === 'busy') {
    return (
      <View className="gap-3 rounded-card bg-surface p-4">
        <Notice tone="warning">Another phone is sending. Taking over disconnects it.</Notice>
        <ButtonRow>
          <SecondaryButton title="Cancel" onPress={onCancel} />
          <PrimaryButton title="Take over" icon="camera" onPress={onTakeOver} />
        </ButtonRow>
      </View>
    );
  }
  return sending
    ? <DestructiveButton title="Stop" icon="stop" accessibilityLabel="Stop sending" onPress={onStop} />
    : <PrimaryButton title="Go live" icon={'broadcast'} disabled={!ready} onPress={onStart} />;
}
