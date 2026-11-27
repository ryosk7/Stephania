// State indicators (DESIGN.md §3.1, §3.4): always a word next to the colour.
import {useEffect, useRef} from 'react';
import {Animated, StyleSheet, Text, View} from 'react-native';
import {Icon, type IconName} from './icon';
import {Footnote} from './text';
import {color, font, radius, space, toneColor, type, type Tone} from './tokens';

/** Coloured dot + label. `pulse` only for on-air. */
export function StatusPill({tone, label, pulse}: {tone: Tone; label: string; pulse?: boolean}) {
  const opacity = useRef(new Animated.Value(1)).current;
  useEffect(() => {
    if (!pulse) { opacity.setValue(1); return; }
    const loop = Animated.loop(Animated.sequence([
      Animated.timing(opacity, {toValue: 0.35, duration: 700, useNativeDriver: true}),
      Animated.timing(opacity, {toValue: 1, duration: 700, useNativeDriver: true}),
    ]));
    loop.start();
    return () => loop.stop();
  }, [pulse, opacity]);
  return (
    <View style={st.pill} accessibilityRole="text" accessibilityLabel={`Status: ${label}`}>
      <Animated.View style={[st.dot, {backgroundColor: toneColor[tone], opacity}]} />
      <Text style={[type.label, {color: color.text, fontSize: 13}]}>{label}</Text>
    </View>
  );
}

/** Red LIVE tag with elapsed time. */
export function LiveBadge({elapsed}: {elapsed?: string}) {
  return (
    <View style={st.live} accessibilityLabel={elapsed ? `Live, ${elapsed}` : 'Live'}>
      <Text style={st.liveTag}>LIVE</Text>
      {elapsed && <Text style={[type.headline, {color: color.text, fontVariant: ['tabular-nums']}]}>{elapsed}</Text>}
    </View>
  );
}

const noticeIcon: Record<Tone, IconName> = {neutral: 'broadcast', live: 'broadcast', warning: 'warning', success: 'done', info: 'broadcast'};

/** Inline sentence with an icon: degradation, errors, hints (DESIGN.md §3.5). */
export function Notice({tone, children, icon}: {tone: Tone; children: string; icon?: IconName}) {
  return (
    <View style={[st.notice, tone !== 'neutral' && {borderColor: toneColor[tone] + '55', backgroundColor: toneColor[tone] + '14'}]} accessibilityRole="text">
      <Icon name={icon ?? noticeIcon[tone]} size={16} tone={tone} />
      <Footnote tone={tone === 'neutral' ? 'secondary' : 'primary'} style={{flex: 1}}>{children}</Footnote>
    </View>
  );
}

const st = StyleSheet.create({
  pill: {flexDirection: 'row', alignItems: 'center', gap: space.s, alignSelf: 'flex-start', height: 28, backgroundColor: color.fill, borderWidth: 1, borderColor: color.border, borderRadius: radius.pill, paddingLeft: 10, paddingRight: space.m},
  dot: {width: 8, height: 8, borderRadius: 4},
  live: {flexDirection: 'row', alignItems: 'center', gap: space.s},
  liveTag: {fontFamily: font.bold, fontSize: 12, lineHeight: 22, letterSpacing: 1, color: '#FFFFFF', backgroundColor: color.live, borderRadius: 6, paddingHorizontal: 8, overflow: 'hidden'},
  notice: {flexDirection: 'row', alignItems: 'flex-start', gap: space.s, borderWidth: 1, borderColor: color.border, borderRadius: radius.control, paddingHorizontal: space.m, paddingVertical: 10},
});
