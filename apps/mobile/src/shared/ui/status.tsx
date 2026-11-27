// State indicators (DESIGN.md §3.1, §3.7): an icon on the state colour plus,
// where there is one, a value (elapsed time). The state word is the spoken
// label and is shown on long press, never only the colour.
import * as Haptics from 'expo-haptics';
import {useEffect, useRef, useState} from 'react';
import {Animated, Pressable, Text, View} from 'react-native';
import {Icon, type IconName} from './icon';
import {Footnote} from './text';
import {toneBg, type Tone} from './tone';

/** Status chip: icon on a tinted dot, optional value. `pulse` only for on-air. */
export function StatusChip({tone, icon, label, value, pulse}: {tone: Tone; icon: IconName; label: string; value?: string; pulse?: boolean}) {
  const opacity = useRef(new Animated.Value(1)).current;
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!pulse) { opacity.setValue(1); return; }
    const loop = Animated.loop(Animated.sequence([
      Animated.timing(opacity, {toValue: 0.45, duration: 700, useNativeDriver: true}),
      Animated.timing(opacity, {toValue: 1, duration: 700, useNativeDriver: true}),
    ]));
    loop.start();
    return () => loop.stop();
  }, [pulse, opacity]);
  useEffect(() => {
    if (!open) return;
    const t = setTimeout(() => setOpen(false), 1600);
    return () => clearTimeout(t);
  }, [open]);
  const text = open ? label : value;
  return (
    <Pressable accessibilityRole="text" accessibilityLabel={value ? `${label}, ${value}` : label}
      onLongPress={() => { void Haptics.selectionAsync(); setOpen(true); }}
      className={`h-[30px] flex-row items-center gap-2 self-start rounded-full border border-border bg-fill pl-1 ${text ? 'pr-3' : 'pr-1'}`}>
      <Animated.View style={{opacity}} className={`h-[22px] w-[22px] items-center justify-center rounded-full ${toneBg[tone]}`}>
        <Icon name={icon} size={13} strokeWidth={2.4} color={tone === 'warning' ? 'on-warning' : 'white'} />
      </Animated.View>
      {text ? <Text className="font-inter-semibold text-label text-text" style={{fontVariant: ['tabular-nums']}}>{text}</Text> : null}
    </Pressable>
  );
}

/** Red LIVE tag with elapsed time. */
export function LiveBadge({elapsed}: {elapsed?: string}) {
  return (
    <View className="flex-row items-center gap-2" accessible accessibilityLabel={elapsed ? `Live, ${elapsed}` : 'Live'}>
      <Text className="overflow-hidden rounded-md bg-live px-2 font-inter-bold text-[12px] leading-[22px] tracking-[1px] text-white">LIVE</Text>
      {elapsed && <Text className="font-inter-semibold text-headline text-text" style={{fontVariant: ['tabular-nums']}}>{elapsed}</Text>}
    </View>
  );
}

const noticeIcon: Record<Tone, IconName> = {neutral: 'info', live: 'broadcast', warning: 'warning', success: 'done', info: 'info'};
const noticeBox: Record<Tone, string> = {
  neutral: 'border-border bg-fill',
  live: 'border-live/40 bg-live/10',
  warning: 'border-warning/40 bg-warning/10',
  success: 'border-success/40 bg-success/10',
  info: 'border-info/40 bg-info/10',
};

/** Short line with an icon: degradation, errors, hints (DESIGN.md §3.5). */
export function Notice({tone, children, icon}: {tone: Tone; children: string; icon?: IconName}) {
  return (
    <View className={`flex-row items-center gap-2.5 rounded-control border px-3 py-2.5 ${noticeBox[tone]}`} accessible accessibilityRole="text">
      <Icon name={icon ?? noticeIcon[tone]} size={18} tone={tone === 'neutral' ? undefined : tone} color={tone === 'neutral' ? 'secondary' : undefined} />
      <Footnote tone={tone === 'neutral' ? 'secondary' : 'primary'} className="flex-1 font-inter-medium">{children}</Footnote>
    </View>
  );
}
