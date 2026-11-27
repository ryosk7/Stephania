// Send this phone's camera(s) to the T3Box (docs/PHONE.md). The screen only
// renders CameraSession state; capture, encoding and transport are native.
import {Stack, useLocalSearchParams} from 'expo-router';
import {useKeepAwake} from 'expo-keep-awake';
import {useEffect, useMemo, useState} from 'react';
import {AppState, Pressable, StyleSheet, Text, View} from 'react-native';
import NativeCamera, {CameraPreview, type CameraMode, type Capabilities} from '../../modules/t3box-camera';
import PinnedFetch from '../../modules/t3box-pinned-fetch';
import {CameraSession, type Snapshot} from '../camera/session';
import {bestDual} from '../camera/plan';
import {DeviceClient} from '../net/device';
import {loadEnrollment} from '../state/session';

const C = {bg: '#0b0b0d', panel: '#17171b', line: '#2a2a31', text: '#f4f4f6', dim: '#9a9aa6', live: '#ff3b30', ok: '#30d158', warn: '#ffd60a', info: '#64d2ff'};

const STATUS: Record<Snapshot['state'], {label: string; color: string}> = {
  idle: {label: 'Ready', color: C.dim},
  starting: {label: 'Opening camera…', color: C.info},
  connecting: {label: 'Connecting…', color: C.info},
  live: {label: 'Live on T3Box', color: C.live},
  reconnecting: {label: 'Reconnecting…', color: C.warn},
  busy: {label: 'Another phone is connected', color: C.warn},
  paused: {label: 'Paused', color: C.dim},
  error: {label: 'Stopped', color: C.dim},
};

const THERMAL: Record<Snapshot['thermal'], string | null> = {
  nominal: null,
  fair: 'Phone is warm: quality slightly reduced',
  serious: 'Phone is hot: quality reduced',
  critical: 'Phone is very hot: quality at minimum',
};

const MODES: {mode: CameraMode; label: string}[] = [
  {mode: 'rear', label: 'Rear'},
  {mode: 'front', label: 'Front'},
  {mode: 'dual', label: 'Both'},
];

export default function CameraScreen() {
  useKeepAwake();
  const {device} = useLocalSearchParams<{device: string}>();
  const [session, setSession] = useState<CameraSession>();
  const [snap, setSnap] = useState<Snapshot>();
  const [caps, setCaps] = useState<Capabilities>();
  const [mode, setMode] = useState<CameraMode>('rear');
  const [setupError, setSetupError] = useState('');

  useEffect(() => {
    let s: CameraSession | undefined;
    let unsub = () => {};
    loadEnrollment(device).then(e => {
      if (!e) { setSetupError('This phone has no access to this T3Box.'); return; }
      s = new CameraSession(new DeviceClient(e, PinnedFetch), NativeCamera);
      unsub = s.subscribe(setSnap);
      setSession(s);
    }).catch(e => setSetupError(String(e)));
    NativeCamera.capabilities().then(setCaps).catch(() => setCaps({roles: [], dualProfiles: [], hardwareH264: false}));
    return () => { unsub(); void s?.release().finally(() => s?.dispose()); };
  }, [device]);

  // iOS stops the cameras in the background; the session pauses and resumes.
  useEffect(() => {
    if (!session) return;
    const sub = AppState.addEventListener('change', st => void session.setBackground(st !== 'active'));
    return () => sub.remove();
  }, [session]);

  // Live preview of the selected cameras while not sending.
  useEffect(() => { if (session && caps) void session.preview(mode); }, [session, caps, mode]);

  const dual = caps ? bestDual(caps) : null;
  const state = snap?.state ?? 'idle';
  const sending = !['idle', 'busy', 'error'].includes(state);
  const shownMode = sending ? snap!.mode : mode;
  const roles = shownMode === 'dual' ? (['rear', 'front'] as const) : ([shownMode] as const);
  const status = STATUS[state];
  const thermal = snap ? THERMAL[snap.thermal] : null;
  const hint = useMemo(() => {
    if (setupError) return setupError;
    if (snap?.message) return snap.message;
    if (state === 'live' && snap?.degraded) return 'Front camera paused to protect the phone. It comes back when the phone cools down.';
    if (!sending) return 'Hold the phone sideways: the T3Box receives a landscape picture.';
    return '';
  }, [setupError, snap, state, sending]);

  return (
    <View style={st.screen}>
      <Stack.Screen options={{title: 'Camera', headerStyle: {backgroundColor: C.bg}, headerTintColor: C.text}} />

      <View style={st.statusRow}>
        <View style={[st.pill, {borderColor: status.color}]} accessibilityRole="text" accessibilityLabel={`Status: ${status.label}`}>
          <View style={[st.dot, {backgroundColor: status.color}, state === 'live' && st.dotLive]} />
          <Text style={st.pillText}>{status.label}{state === 'reconnecting' && snap ? ` (${snap.attempt})` : ''}</Text>
        </View>
        {thermal && <Text style={st.thermal} numberOfLines={2}>{thermal}</Text>}
      </View>

      <View style={st.previews}>
        {roles.map((role, i) => (
          <View key={role} style={[st.frame, roles.length === 2 && i === 1 && st.frameSecondary]}>
            {caps?.roles.includes(role)
              ? <CameraPreview camera={role} mirrored={role === 'front'} style={StyleSheet.absoluteFill} />
              : <Text style={st.placeholder}>No {role} camera</Text>}
            <View style={st.label}>
              <Text style={st.labelText}>{role === 'rear' ? 'Presenter · rear' : 'Audience · front'}</Text>
              {sending && snap?.degraded && role === 'front' && <Text style={[st.labelText, {color: C.warn}]}> · paused</Text>}
            </View>
          </View>
        ))}
      </View>

      <View style={st.panel}>
        <View style={st.segment} accessibilityRole="radiogroup">
          {MODES.map(m => {
            const unavailable = m.mode === 'dual' ? !dual : !caps?.roles.includes(m.mode);
            const selected = shownMode === m.mode;
            return (
              <Pressable key={m.mode} accessibilityRole="radio" accessibilityState={{selected, disabled: unavailable || sending}}
                disabled={unavailable || sending} onPress={() => setMode(m.mode)}
                style={[st.segmentItem, selected && st.segmentSelected, (unavailable || sending) && !selected && {opacity: 0.35}]}>
                <Text style={[st.segmentText, selected && {color: C.bg}]}>{m.label}</Text>
              </Pressable>
            );
          })}
        </View>
        <Text style={st.caption}>
          {caps && !dual ? 'This phone cannot use both cameras at once.' : dual ? `Both cameras: up to ${dual.split('+')[0]}` : ' '}
        </Text>

        {state === 'busy' ? (
          <View style={st.busy}>
            <Text style={st.busyText}>Another phone is sending to this T3Box. Taking over disconnects it.</Text>
            <View style={{flexDirection: 'row', gap: 10}}>
              <Action title="Cancel" kind="secondary" onPress={() => void session?.stop()} />
              <Action title="Take over" kind="primary" onPress={() => void session?.takeOver()} />
            </View>
          </View>
        ) : (
          <View style={{flexDirection: 'row'}}>
            <Action title={sending ? 'Stop sending' : 'Go live'} kind={sending ? 'stop' : 'primary'} disabled={!session}
              onPress={() => void (sending ? session!.stop() : session!.start(mode))} />
          </View>
        )}
        {hint !== '' && <Text style={st.hint}>{hint}</Text>}
      </View>
    </View>
  );
}

function Action({title, onPress, kind, disabled}: {title: string; onPress: () => void; kind: 'primary' | 'secondary' | 'stop'; disabled?: boolean}) {
  const bg = kind === 'stop' ? C.live : kind === 'primary' ? C.text : C.panel;
  return (
    <Pressable accessibilityRole="button" disabled={disabled} onPress={onPress}
      style={({pressed}) => [st.action, {backgroundColor: bg, opacity: disabled ? 0.4 : pressed ? 0.75 : 1}, kind === 'secondary' && {borderWidth: 1, borderColor: C.line}]}>
      <Text style={[st.actionText, {color: kind === 'primary' ? C.bg : C.text}]}>{title}</Text>
    </Pressable>
  );
}

const st = StyleSheet.create({
  screen: {flex: 1, backgroundColor: C.bg, padding: 16, gap: 14},
  statusRow: {flexDirection: 'row', alignItems: 'center', gap: 12, minHeight: 34},
  pill: {flexDirection: 'row', alignItems: 'center', gap: 8, borderWidth: 1, borderRadius: 999, paddingHorizontal: 12, paddingVertical: 6},
  dot: {width: 8, height: 8, borderRadius: 4},
  dotLive: {shadowColor: C.live, shadowOpacity: 0.9, shadowRadius: 6, shadowOffset: {width: 0, height: 0}},
  pillText: {color: C.text, fontSize: 14, fontWeight: '600'},
  thermal: {flex: 1, color: C.warn, fontSize: 13},
  previews: {flex: 1, gap: 10, justifyContent: 'center'},
  frame: {width: '100%', aspectRatio: 16 / 9, borderRadius: 14, overflow: 'hidden', backgroundColor: C.panel, alignItems: 'center', justifyContent: 'center'},
  frameSecondary: {width: '62%', alignSelf: 'flex-end'},
  placeholder: {color: C.dim, fontSize: 14},
  label: {position: 'absolute', left: 10, bottom: 8, flexDirection: 'row', backgroundColor: 'rgba(0,0,0,0.55)', borderRadius: 6, paddingHorizontal: 8, paddingVertical: 3},
  labelText: {color: C.text, fontSize: 12, fontWeight: '600'},
  panel: {backgroundColor: C.panel, borderRadius: 18, padding: 14, gap: 10},
  segment: {flexDirection: 'row', backgroundColor: C.bg, borderRadius: 12, padding: 3},
  segmentItem: {flex: 1, paddingVertical: 9, borderRadius: 9, alignItems: 'center'},
  segmentSelected: {backgroundColor: C.text},
  segmentText: {color: C.text, fontSize: 15, fontWeight: '600'},
  caption: {color: C.dim, fontSize: 12, textAlign: 'center'},
  action: {flex: 1, paddingVertical: 15, borderRadius: 14, alignItems: 'center'},
  actionText: {fontSize: 17, fontWeight: '700'},
  busy: {gap: 10},
  busyText: {color: C.text, fontSize: 14, lineHeight: 20},
  hint: {color: C.dim, fontSize: 13, lineHeight: 18, textAlign: 'center'},
});
