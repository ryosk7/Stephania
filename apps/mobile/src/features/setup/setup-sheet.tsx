// The setup sheet (DESIGN.md §6): the device as a picture, its name and one
// "Set up" button. Several devices are the same full-size card paged
// sideways, the chosen one centred, the next peeking in, dots below. With
// `autoStart` (iOS 18: the system sheet already asked) setup runs at once.
import {router} from 'expo-router';
import {useEffect, useRef, useState} from 'react';
import {ActivityIndicator, ScrollView, View, type LayoutChangeEvent, type NativeScrollEvent, type NativeSyntheticEvent} from 'react-native';
import {Footnote, Notice, PrimaryButton, Sheet, Title, useColor} from '@/shared/ui';
import {DeviceArt} from './device-art';
import {describeSetupError} from './errors';
import {finishSetup, openDevice} from './flow';
import type {NearbyDevice} from './use-nearby';

const CARD = 220, GAP = 20;

type Phase = {kind: 'idle'} | {kind: 'busy'; label: string} | {kind: 'error'; message: string};

export function SetupSheet({visible, devices, autoStart = false, scanError = '', onClose}: {
  visible: boolean; devices: NearbyDevice[]; autoStart?: boolean; scanError?: string; onClose(): void;
}) {
  const [index, setIndex] = useState(0);
  const [phase, setPhase] = useState<Phase>({kind: 'idle'});
  const [width, setWidth] = useState(0);
  const started = useRef(false);
  const spinner = useColor('secondary');
  const device = devices[Math.min(index, devices.length - 1)];

  useEffect(() => { if (!visible) { started.current = false; setPhase({kind: 'idle'}); setIndex(0); } }, [visible]);
  useEffect(() => {
    if (visible && autoStart && device && !started.current) { started.current = true; void setUp(device); }
  });

  async function setUp(d: NearbyDevice) {
    setPhase({kind: 'busy', label: `Setting up ${d.name}`});
    try {
      const {online} = await openDevice(d.id, d.name);
      if (online) {
        setPhase({kind: 'busy', label: 'Setting up secure access'});
        const id = await finishSetup();
        onClose();
        router.push({pathname: '/device/[id]', params: {id}});
      } else {
        onClose();
        router.push('/setup/wifi');
      }
    } catch (e) {
      setPhase({kind: 'error', message: describeSetupError(e)});
    }
  }

  const busy = phase.kind === 'busy';
  const onScroll = (e: NativeSyntheticEvent<NativeScrollEvent>) =>
    setIndex(Math.max(0, Math.min(devices.length - 1, Math.round(e.nativeEvent.contentOffset.x / (CARD + GAP)))));
  const card = (d: NearbyDevice, i: number) => (
    <View key={d.id} style={{width: CARD, opacity: i === index ? 1 : 0.45}} className="items-center gap-3.5"
      accessible accessibilityRole="button" accessibilityLabel={d.name} accessibilityState={{selected: i === index}}>
      <DeviceArt />
      <View className="items-center gap-1">
        <Title className="text-center" numberOfLines={1}>{d.name}</Title>
        <Footnote className="text-center">Stephania Connect</Footnote>
      </View>
    </View>
  );

  return (
    <Sheet visible={visible} onClose={busy ? () => {} : onClose} bare>
      {devices.length === 0 ? (
        <View className="items-center gap-3.5 py-2">
          <DeviceArt />
          <View className="flex-row items-center gap-2"><ActivityIndicator color={spinner} /><Footnote>Looking for a Stephania device…</Footnote></View>
        </View>
      ) : devices.length === 1 ? (
        <View className="items-center">{card(devices[0]!, 0)}</View>
      ) : (
        <>
          <ScrollView horizontal showsHorizontalScrollIndicator={false} decelerationRate="fast" snapToInterval={CARD + GAP}
            scrollEnabled={!busy} onScroll={onScroll} scrollEventThrottle={32} className="-mx-5"
            onLayout={(e: LayoutChangeEvent) => setWidth(e.nativeEvent.layout.width)}
            contentContainerStyle={{gap: GAP, paddingHorizontal: Math.max(0, (width - CARD) / 2)}}
            accessibilityLabel="Stephania devices nearby">
            {devices.map(card)}
          </ScrollView>
          <View className="flex-row items-center justify-center gap-1.5" accessibilityElementsHidden>
            {devices.map((d, i) => <View key={d.id} className={`h-1.5 rounded-full ${i === index ? 'w-[18px] bg-text' : 'w-1.5 bg-tertiary'}`} />)}
          </View>
        </>
      )}
      {(phase.kind === 'error' || scanError !== '') && <Notice tone="warning">{phase.kind === 'error' ? phase.message : scanError}</Notice>}
      {busy && <Footnote className="text-center">{phase.label}…</Footnote>}
      {device && (
        <PrimaryButton title="Set up" busy={busy} accessibilityLabel={`Set up ${device.name}`} onPress={() => void setUp(device)} />
      )}
    </Sheet>
  );
}
