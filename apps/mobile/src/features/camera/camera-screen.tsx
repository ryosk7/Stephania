// Send this phone's camera(s) to the T3Box (docs/PHONE.md; DESIGN.md §6
// Camera). Renders CameraSession state only; capture, encoding and
// transport are native.
import * as Haptics from 'expo-haptics';
import {router} from 'expo-router';
import {useEffect, useState} from 'react';
import {StyleSheet, View} from 'react-native';
import {CameraPreview, type CameraMode, type CameraRole} from '@modules/t3box-camera';
import {DeviceGate, useDeviceClient, type DeviceClient} from '@/shared/device';
import {ButtonRow, Overline, DestructiveButton, Footnote, IconButton, Screen, LiveBadge, Notice, PrimaryButton, SecondaryButton, Segmented, StatusPill, color, radius, space, type Tone} from '@/shared/ui';
import {bestDual} from './plan';
import type {Snapshot} from './session';
import {useCameraSession, useElapsed} from './use-camera-session';

const STATUS: Record<Snapshot['state'], {label: string; tone: Tone}> = {
  idle: {label: 'Ready', tone: 'neutral'},
  starting: {label: 'Opening camera…', tone: 'info'},
  connecting: {label: 'Connecting…', tone: 'info'},
  live: {label: 'Live', tone: 'live'},
  reconnecting: {label: 'Reconnecting…', tone: 'warning'},
  busy: {label: 'Another phone is connected', tone: 'warning'},
  paused: {label: 'Paused', tone: 'neutral'},
  error: {label: 'Stopped', tone: 'neutral'},
};

const THERMAL: Record<Snapshot['thermal'], string | null> = {
  nominal: null,
  fair: 'Phone is warm: quality slightly reduced.',
  serious: 'Phone is hot: quality reduced.',
  critical: 'Phone is very hot: quality at minimum.',
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
      <View style={st.top}>
        <IconButton icon={'close'} label="Close" onPress={() => router.back()} />
        {state === 'live' ? <LiveBadge elapsed={elapsed} /> : <StatusPill tone={status.tone} label={state === 'reconnecting' && snap ? `${status.label} (${snap.attempt})` : status.label} />}
      </View>
      {thermal && <Notice tone="warning" icon={'thermometer'}>{thermal}</Notice>}

      <View style={st.previews}>
        {roles.map((role, i) => (
          <View key={role} style={[st.frame, roles.length === 2 && i === 1 && st.frameSecondary]}>
            {caps?.roles.includes(role)
              ? <CameraPreview camera={role} mirrored={role === 'front'} style={StyleSheet.absoluteFill} />
              : <Footnote>No {role} camera</Footnote>}
            <View style={st.label}>
              <Overline tone="primary">{role === 'rear' ? 'Presenter · rear' : 'Audience · front'}</Overline>
              {sending && snap?.degraded && role === 'front' && <Overline tone="warning"> · paused</Overline>}
            </View>
          </View>
        ))}
      </View>

      <View style={{gap: space.s}}>
        <Segmented label="Cameras" value={shownMode} onChange={setMode} locked={sending}>
          <Segmented.Item value="rear" label="Rear" unavailable={caps ? !caps.roles.includes('rear') : true} />
          <Segmented.Item value="front" label="Front" unavailable={caps ? !caps.roles.includes('front') : true} />
          <Segmented.Item value="dual" label="Both" unavailable={!dual} />
        </Segmented>
        <Footnote style={{textAlign: 'center'}}>
          {caps && !dual ? 'This phone cannot use both cameras at once.' : dual ? `Both cameras up to ${dual.replace('+', ' + ')}` : ' '}
        </Footnote>
        {message !== '' && <Notice tone={state === 'error' ? 'warning' : 'neutral'}>{message}</Notice>}
        {!sending && message === '' && <Footnote style={{textAlign: 'center'}}>Hold the phone sideways: the T3Box receives a landscape picture.</Footnote>}
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
      <View style={st.sheet}>
        <Notice tone="warning">Another phone is sending to this T3Box. Taking over disconnects it.</Notice>
        <ButtonRow>
          <SecondaryButton title="Cancel" onPress={onCancel} />
          <PrimaryButton title="Take over" onPress={onTakeOver} />
        </ButtonRow>
      </View>
    );
  }
  return sending
    ? <DestructiveButton title="Stop sending" onPress={onStop} />
    : <PrimaryButton title="Go live" icon={'broadcast'} disabled={!ready} onPress={onStart} />;
}

const st = StyleSheet.create({
  top: {flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', minHeight: 44},
  previews: {flex: 1, gap: space.m, justifyContent: 'center'},
  frame: {width: '100%', aspectRatio: 16 / 9, borderRadius: radius.card, overflow: 'hidden', backgroundColor: color.surface, alignItems: 'center', justifyContent: 'center'},
  frameSecondary: {width: '62%', alignSelf: 'flex-end'},
  label: {position: 'absolute', left: space.s + 2, bottom: space.s, flexDirection: 'row', backgroundColor: color.scrim, borderRadius: 6, paddingHorizontal: space.s, paddingVertical: 3},
  sheet: {backgroundColor: color.surface, borderRadius: radius.card, padding: space.l, gap: space.m},
});
