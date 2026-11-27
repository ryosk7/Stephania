// Operating one T3Box over the LAN (DESIGN.md §6 Control): device name and
// broadcast state on top, layouts as tiles (the on-air one marked), audio,
// and the stream action pinned at the bottom.
import * as Haptics from 'expo-haptics';
import {Stack, router} from 'expo-router';
import {useState} from 'react';
import {Alert, Pressable, StyleSheet, View} from 'react-native';
import {DeviceGate, broadcastState, useDeviceClient, type DeviceClient, type StoredScene} from '@/shared/device';
import {
  Caption, Card, DestructiveButton, Footnote, Headline, InlineIconButton, LargeTitle, List, ListIcon, Notice, PrimaryButton, Screen,
  StatusPill, Subheadline, Toggle, color, radius, space, symbols,
} from '@/shared/ui';
import {RenameSheet} from './rename-sheet';
import {SceneThumbnail} from './scene-thumbnail';
import {useControl} from './use-control';

export function ControlScreen({deviceId}: {deviceId: string}) {
  const handle = useDeviceClient(deviceId);
  return (
    <DeviceGate handle={handle}>
      {handle.state === 'ready' && <Control deviceId={deviceId} name={handle.name} host={handle.host} client={handle.client} />}
    </DeviceGate>
  );
}

function Control({deviceId, name: initialName, host, client}: {deviceId: string; name: string; host: string; client: DeviceClient}) {
  const {status, scenes, audio, error, pending, run} = useControl(client);
  const [fade, setFade] = useState(true);
  const [name, setName] = useState(initialName);
  const [renaming, setRenaming] = useState(false);
  const state = broadcastState(status);
  const streaming = status?.streaming_requested === true;
  const onAirId = status?.program?.scene_id;

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
        <View style={st.titleRow}>
          <LargeTitle style={{flexShrink: 1}} numberOfLines={1}>{name}</LargeTitle>
          <InlineIconButton symbol={symbols.pencil} label="Rename" onPress={() => setRenaming(true)} />
        </View>
        {host !== '' && <Subheadline>{host}</Subheadline>}
        <View style={st.statusLine}>
          <StatusPill tone={state.tone} label={state.label} pulse={state.pulse} />
          {status?.last_error ? <Footnote tone="warning" style={{flex: 1}} numberOfLines={2}>{status.last_error}</Footnote> : null}
        </View>
      </View>
      {error !== '' && <Notice tone="warning">{error}</Notice>}

      <View style={{gap: space.s}}>
        <View style={st.sectionHeader}>
          <View style={{flex: 1}}>
            <Footnote style={{letterSpacing: 0.2}}>LAYOUTS</Footnote>
            <Footnote>{streaming ? 'What viewers see. Tap to switch.' : 'What viewers will see. Switch while streaming.'}</Footnote>
          </View>
          <Footnote>Fade</Footnote>
          <Toggle value={fade} onChange={setFade} label="Switch layouts with a 0.5 second fade" />
        </View>
        {scenes.length === 0 ? (
          <Card>
            <Headline>No layouts yet</Headline>
            <Footnote>The HDMI input goes on air as it is. Create layouts in the Studio on a PC.</Footnote>
          </Card>
        ) : (
          <View style={st.grid}>
            {scenes.map(sc => (
              <LayoutTile key={sc.id} scene={sc} onAir={onAirId === sc.id} disabled={!streaming || pending !== null}
                onPress={() => void run(`scene:${sc.id}`, () => client.transition(sc.id, sc.revision, fade ? 500 : undefined))} />
            ))}
          </View>
        )}
      </View>

      <List>
        <List.Section title="Audio">
          {audio.map(a => (
            <List.Row key={a.id} title={a.id === 'hdmi' ? 'HDMI audio' : 'Microphone'} detail={a.enabled ? (a.muted ? 'Muted' : 'On air') : 'Not connected'}
              leading={<ListIcon symbol={a.id === 'hdmi' ? symbols.speaker : symbols.mic} />}
              accessory={<Toggle value={!a.muted} disabled={pending !== null || !a.enabled}
                label={`${a.id === 'hdmi' ? 'HDMI audio' : 'Microphone'} on air`}
                onChange={on => void run(`audio:${a.id}`, () => client.setMuted(a.id, !on))} />} />
          ))}
        </List.Section>
        <List.Section title="More">
          <List.Row title="Send this phone's camera" leading={<ListIcon symbol={symbols.camera} />}
            onPress={() => router.push({pathname: '/device/[id]/camera', params: {id: deviceId}})} />
          <List.Row title="Open the Studio on a PC" leading={<ListIcon symbol={symbols.desktop} />}
            onPress={() => router.push({pathname: '/device/[id]/pc', params: {id: deviceId}})} />
        </List.Section>
      </List>

      <RenameSheet deviceId={deviceId} name={name} host={host} visible={renaming} onClose={() => setRenaming(false)} onRenamed={setName} />
    </Screen>
  );
}

/**
 * Layout tile: a 16:9 miniature with the name under it. Always half the row,
 * whatever the count, so 1, 3 or 9 layouts look the same; the on-air tile is
 * outlined red with an ON AIR tag (DESIGN.md §2).
 */
function LayoutTile({scene, onAir, disabled, onPress}: {scene: StoredScene; onAir: boolean; disabled: boolean; onPress: () => void}) {
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={`${scene.document.name}${onAir ? ', on air' : ''}`}
      accessibilityState={{disabled, selected: onAir}} disabled={disabled || onAir} onPress={onPress}
      style={({pressed}) => [st.tile, {opacity: disabled && !onAir ? 0.5 : pressed ? 0.7 : 1}]}>
      <View style={[st.thumb, onAir && st.thumbOnAir]}>
        <SceneThumbnail layers={scene.document.layers} />
        {onAir && <Caption tone="primary" style={st.onAirTag}>ON AIR</Caption>}
      </View>
      <Headline numberOfLines={2} style={{fontSize: 15, lineHeight: 20, paddingHorizontal: 2}}>{scene.document.name}</Headline>
    </Pressable>
  );
}

const st = StyleSheet.create({
  titleRow: {flexDirection: 'row', alignItems: 'center', gap: space.s},
  statusLine: {flexDirection: 'row', alignItems: 'center', gap: space.m},
  sectionHeader: {flexDirection: 'row', alignItems: 'center', gap: space.s, paddingLeft: space.l},
  grid: {flexDirection: 'row', flexWrap: 'wrap', columnGap: space.m, rowGap: space.l},
  tile: {width: '48%', gap: space.s},
  thumb: {width: '100%', aspectRatio: 16 / 9, borderRadius: radius.tile, backgroundColor: color.surface, borderWidth: 2, borderColor: 'transparent', overflow: 'hidden'},
  thumbOnAir: {borderColor: color.live},
  onAirTag: {position: 'absolute', top: 6, left: 6, backgroundColor: color.live, borderRadius: 5, paddingHorizontal: 6, lineHeight: 18, overflow: 'hidden', fontWeight: '800', letterSpacing: 0.6, fontSize: 10},
});
