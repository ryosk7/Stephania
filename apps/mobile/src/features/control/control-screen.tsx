// Operating one T3Box over the LAN (DESIGN.md §6 Control): name and state
// on top, layouts as tiles (the on-air one marked), audio, and the stream
// action pinned at the bottom. Sections are headed by icons.
import * as Haptics from 'expo-haptics';
import {router} from 'expo-router';
import {useState} from 'react';
import {Pressable, View} from 'react-native';
import {DeviceGate, broadcastState, useDeviceClient, useOnAirTime, type DeviceClient, type StoredScene} from '@/shared/device';
import {
  ButtonRow, Card, DestructiveButton, Display, Footnote, Headline, Icon, IconButton, List, ListIcon, Mono, Notice,
  PrimaryButton, Screen, SecondaryButton, SectionHeading, Sheet, StatusChip, Toggle, TopBar,
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
  const [confirmStop, setConfirmStop] = useState(false);
  const state = broadcastState(status);
  const onAirTime = useOnAirTime(status);
  const streaming = status?.streaming_requested === true;
  const onAirId = status?.program?.scene_id;

  const start = () => void run('stream', async () => {
    await client.start();
    void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
  });

  return (
    <Screen scroll
      header={<TopBar onBack={() => router.back()} trailing={<>
        <IconButton icon="camera" label="Send this phone's camera" onPress={() => router.push({pathname: '/device/[id]/camera', params: {id: deviceId}})} />
        <IconButton icon="desktop" label="Open the Studio on a PC" onPress={() => router.push({pathname: '/device/[id]/pc', params: {id: deviceId}})} />
      </>} />}
      footer={streaming
        ? <DestructiveButton title="Stop" icon="stop" accessibilityLabel="Stop stream" busy={pending === 'stream'} onPress={() => setConfirmStop(true)} />
        : <PrimaryButton title="Go live" icon="broadcast" accessibilityLabel="Start stream" busy={pending === 'stream'} disabled={!status} onPress={start} />}>
      <View className="gap-1.5">
        <View className="flex-row items-center gap-2.5">
          <Display className="shrink" numberOfLines={1}>{name}</Display>
          <IconButton shape="inline" icon="pencil" label="Rename" onPress={() => setRenaming(true)} />
        </View>
        {host !== '' && <Mono tone="secondary">{host}</Mono>}
        <View className="mt-1 flex-row items-center gap-3">
          <StatusChip tone={state.tone} icon={state.icon} label={state.label} value={onAirTime} pulse={state.pulse} />
          {status?.last_error ? <Footnote tone="warning" className="flex-1" numberOfLines={2}>{status.last_error}</Footnote> : null}
        </View>
      </View>
      {error !== '' && <Notice tone="warning">{error}</Notice>}

      <View className="gap-2.5">
        <SectionHeading icon="layouts" label="Layouts" aside={
          <View className="flex-row items-center gap-2.5">
            <Icon name="fade" size={18} color="secondary" />
            <Toggle value={fade} onChange={setFade} label="Fade between layouts (0.5 seconds)" />
          </View>} />
        {scenes.length === 0 ? (
          <Card>
            <View className="flex-row items-center gap-3" accessible accessibilityLabel="No layouts yet. Make layouts in the Studio on a PC.">
              <ListIcon icon="desktop" />
              <Headline className="flex-1">Make layouts in the Studio</Headline>
            </View>
          </Card>
        ) : (
          <View className="flex-row flex-wrap gap-x-3 gap-y-4">
            {scenes.map(sc => (
              <LayoutTile key={sc.id} scene={sc} onAir={onAirId === sc.id} disabled={!streaming || pending !== null}
                onPress={() => void run(`scene:${sc.id}`, () => client.transition(sc.id, sc.revision, fade ? 500 : undefined))} />
            ))}
          </View>
        )}
      </View>

      <List.Section heading={<SectionHeading icon="speaker" label="Audio" />}>
        {audio.map(a => {
          const hdmi = a.id === 'hdmi';
          const on = a.enabled && !a.muted;
          return (
            <List.Row key={a.id} title={hdmi ? 'HDMI' : 'Mic'} detail={a.enabled ? undefined : 'Not connected'}
              leading={<ListIcon icon={hdmi ? (on ? 'speaker' : 'speakerOff') : (on ? 'mic' : 'micOff')} color={on ? 'text' : 'tertiary'} />}
              accessory={<Toggle value={!a.muted} disabled={pending !== null || !a.enabled}
                label={`${hdmi ? 'HDMI audio' : 'Microphone'} on air`}
                onChange={v => void run(`audio:${a.id}`, () => client.setMuted(a.id, !v))} />} />
          );
        })}
      </List.Section>

      <Sheet visible={confirmStop} title="Stop the stream?" icon="stop" onClose={() => setConfirmStop(false)}>
        <Footnote>Viewers will see the stream end.</Footnote>
        <ButtonRow>
          <SecondaryButton title="Keep" accessibilityLabel="Keep streaming" onPress={() => setConfirmStop(false)} />
          <DestructiveButton title="Stop" icon="stop" accessibilityLabel="Stop stream" onPress={() => { setConfirmStop(false); void run('stream', () => client.stop()); }} />
        </ButtonRow>
      </Sheet>
      <RenameSheet deviceId={deviceId} name={name} host={host} visible={renaming} onClose={() => setRenaming(false)} onRenamed={setName} />
    </Screen>
  );
}

/**
 * Layout tile: a 16:9 miniature with the name under it. Always half the row,
 * whatever the count, so 1, 3 or 9 layouts look the same; the on-air tile is
 * outlined red with the broadcast mark.
 */
function LayoutTile({scene, onAir, disabled, onPress}: {scene: StoredScene; onAir: boolean; disabled: boolean; onPress: () => void}) {
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={`${scene.document.name}${onAir ? ', on air' : ''}`}
      accessibilityState={{disabled, selected: onAir}} disabled={disabled || onAir} onPress={onPress}
      className={`w-[48%] gap-2 active:opacity-70 ${disabled && !onAir ? 'opacity-50' : ''}`}>
      <View className={`aspect-video w-full overflow-hidden rounded-tile bg-surface ${onAir ? 'border-2 border-live' : 'border border-border'}`}>
        <SceneThumbnail layers={scene.document.layers} />
        {onAir && (
          <View className="absolute left-1.5 top-1.5 h-[22px] w-[22px] items-center justify-center rounded-full bg-live">
            <Icon name="broadcast" size={13} strokeWidth={2.4} color="white" />
          </View>
        )}
      </View>
      <Headline numberOfLines={2} className="px-0.5 font-inter-medium text-label">{scene.document.name}</Headline>
    </Pressable>
  );
}
