// Operating one T3Box over the LAN (DESIGN.md §6 Control): device name and
// broadcast state on top, what is on air, scenes as tiles, audio, and the
// stream action pinned at the bottom.
import * as Haptics from 'expo-haptics';
import {Stack, router} from 'expo-router';
import {useState} from 'react';
import {Alert, Pressable, StyleSheet, Switch, View} from 'react-native';
import {DeviceGate, broadcastState, useDeviceClient, type DeviceClient, type StoredScene} from '@/shared/device';
import {
  Body, Caption, Card, DestructiveButton, Footnote, Headline, LargeTitle, List, Notice, PrimaryButton, Screen,
  StatusPill, Symbol, color, radius, space, symbols,
} from '@/shared/ui';
import {useControl} from './use-control';

export function ControlScreen({deviceId}: {deviceId: string}) {
  const handle = useDeviceClient(deviceId);
  return <DeviceGate handle={handle}>{handle.state === 'ready' && <Control deviceId={deviceId} name={handle.name} client={handle.client} />}</DeviceGate>;
}

function Control({deviceId, name, client}: {deviceId: string; name: string; client: DeviceClient}) {
  const {status, scenes, audio, error, pending, run} = useControl(client);
  const [fade, setFade] = useState(true);
  const state = broadcastState(status);
  const streaming = status?.streaming_requested === true;
  const onAir = status?.program ? scenes.find(s => s.id === status.program!.scene_id) : undefined;

  const stop = () => Alert.alert('Stop the stream?', 'Viewers will see the stream end.', [
    {text: 'Cancel', style: 'cancel'},
    {text: 'Stop stream', style: 'destructive', onPress: () => void run('stream', () => client.stop())},
  ]);
  const start = () => void run('stream', async () => {
    await client.start();
    void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
  });

  return (
    <Screen scroll footer={streaming
      ? <DestructiveButton title="Stop stream" busy={pending === 'stream'} onPress={stop} />
      : <PrimaryButton title="Start stream" symbol={symbols.broadcast} busy={pending === 'stream'} disabled={!status} onPress={start} />}>
      <Stack.Screen options={{title: ''}} />
      <View style={{gap: space.s}}>
        <LargeTitle>{name}</LargeTitle>
        <View style={st.statusLine}>
          <StatusPill tone={state.tone} label={state.label} pulse={state.pulse} />
          {status?.last_error ? <Footnote tone="warning" style={{flex: 1}} numberOfLines={2}>{status.last_error}</Footnote> : null}
        </View>
      </View>
      {error !== '' && <Notice tone="warning">{error}</Notice>}

      <Card>
        <Caption>ON AIR</Caption>
        <Headline>{onAir ? onAir.document.name : status?.program ? status.program.scene_id : 'HDMI input (no scene)'}</Headline>
        {status?.program && <Footnote>Revision {status.program.revision}</Footnote>}
        <View style={st.switchRow}>
          <Body style={{flex: 1}}>Switch scenes with a 0.5 s fade</Body>
          <Switch value={fade} onValueChange={setFade} accessibilityLabel="Switch scenes with a fade" />
        </View>
      </Card>

      <View style={{gap: space.s}}>
        <Caption style={{paddingHorizontal: space.l}}>SCENES</Caption>
        {!streaming && scenes.length > 0 && <Footnote style={{paddingHorizontal: space.l}}>Scenes can be switched while streaming.</Footnote>}
        <View style={st.grid}>
          {scenes.map(sc => (
            <SceneTile key={sc.id} scene={sc} onAir={status?.program?.scene_id === sc.id} disabled={!streaming || pending !== null}
              onPress={() => void run(`scene:${sc.id}`, () => client.transition(sc.id, sc.revision, fade ? 500 : undefined))} />
          ))}
        </View>
        {scenes.length === 0 && <Footnote style={{paddingHorizontal: space.l}}>No scenes yet. Create them in the Studio on a PC.</Footnote>}
      </View>

      <List>
        <List.Section title="Audio">
          {audio.map(a => (
            <List.Row key={a.id} title={a.id === 'hdmi' ? 'HDMI audio' : 'Microphone'} detail={a.enabled ? (a.muted ? 'Muted' : 'On air') : 'Not connected'}
              leading={<Symbol name={a.id === 'hdmi' ? symbols.speaker : symbols.mic} size={16} tint={color.textSecondary} />}
              accessory={<Switch value={!a.muted} disabled={pending !== null || !a.enabled}
                accessibilityLabel={`${a.id === 'hdmi' ? 'HDMI audio' : 'Microphone'} on air`}
                onValueChange={on => void run(`audio:${a.id}`, () => client.setMuted(a.id, !on))} />} />
          ))}
        </List.Section>
        <List.Section title="More">
          <List.Row title="Send this phone's camera" leading={<Symbol name={symbols.camera} size={16} tint={color.textSecondary} />}
            onPress={() => router.push({pathname: '/device/[id]/camera', params: {id: deviceId}})} />
          <List.Row title="Open the Studio on a PC" leading={<Symbol name={symbols.desktop} size={16} tint={color.textSecondary} />}
            onPress={() => router.push({pathname: '/device/[id]/pc', params: {id: deviceId}})} />
        </List.Section>
      </List>
    </Screen>
  );
}

/** 16:9 scene tile; the on-air tile is outlined red, not recoloured (DESIGN.md §2). */
function SceneTile({scene, onAir, disabled, onPress}: {scene: StoredScene; onAir: boolean; disabled: boolean; onPress: () => void}) {
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={`${scene.document.name}${onAir ? ', on air' : ''}`}
      accessibilityState={{disabled, selected: onAir}} disabled={disabled || onAir} onPress={onPress}
      style={({pressed}) => [st.tile, onAir && st.tileOnAir, {opacity: disabled && !onAir ? 0.5 : pressed ? 0.7 : 1}]}>
      <Headline numberOfLines={2}>{scene.document.name}</Headline>
      <Caption>{onAir ? 'On air' : `r${scene.revision}`}</Caption>
    </Pressable>
  );
}

const st = StyleSheet.create({
  statusLine: {flexDirection: 'row', alignItems: 'center', gap: space.m},
  switchRow: {flexDirection: 'row', alignItems: 'center', gap: space.m, marginTop: space.xs},
  grid: {flexDirection: 'row', flexWrap: 'wrap', gap: space.m},
  tile: {width: '47%', flexGrow: 1, aspectRatio: 16 / 9, borderRadius: radius.tile, backgroundColor: color.surface, padding: space.m, justifyContent: 'space-between', borderWidth: 2, borderColor: 'transparent'},
  tileOnAir: {borderColor: color.live},
});
