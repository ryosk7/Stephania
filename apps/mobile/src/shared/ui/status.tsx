// State indicators (DESIGN.md §3.1, §3.4): always a word next to the colour.
import {useEffect, useRef} from 'react';
import {Animated, StyleSheet, Text, View} from 'react-native';
import {Symbol, symbols, type SymbolName} from './symbol';
import {Footnote} from './text';
import {color, radius, space, toneColor, type, type Tone} from './tokens';

/** Coloured dot + label (GoPro "Busy", Tesla status line). `pulse` only for on-air. */
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
      <Text style={[type.footnote, {color: color.text, fontWeight: '600'}]}>{label}</Text>
    </View>
  );
}

/** Red LIVE tag with optional elapsed time (live-app pattern). */
export function LiveBadge({elapsed}: {elapsed?: string}) {
  return (
    <View style={st.live} accessibilityLabel={elapsed ? `Live, ${elapsed}` : 'Live'}>
      <Text style={[type.caption, st.liveText]}>LIVE</Text>
      {elapsed && <Text style={[type.subheadline, {color: color.text, fontWeight: '600', fontVariant: ['tabular-nums']}]}>{elapsed}</Text>}
    </View>
  );
}

const noticeSymbol: Record<Tone, SymbolName> = {
  neutral: symbols.broadcast, live: symbols.broadcast, warning: symbols.warning, success: symbols.check, info: symbols.broadcast,
};

/** Inline sentence with a symbol: degradation, errors, hints (DESIGN.md §3.5). */
export function Notice({tone, children, symbol}: {tone: Tone; children: string; symbol?: SymbolName}) {
  return (
    <View style={st.notice} accessibilityRole="text">
      <Symbol name={symbol ?? noticeSymbol[tone]} size={15} tone={tone} />
      <Footnote tone={tone === 'neutral' ? 'secondary' : tone} style={{flex: 1}}>{children}</Footnote>
    </View>
  );
}

const st = StyleSheet.create({
  pill: {flexDirection: 'row', alignItems: 'center', gap: 7, alignSelf: 'flex-start', height: 28, backgroundColor: color.fill, borderRadius: 14, paddingLeft: 10, paddingRight: space.m},
  dot: {width: 8, height: 8, borderRadius: 4},
  live: {flexDirection: 'row', alignItems: 'center', gap: space.s, alignSelf: 'flex-start'},
  liveText: {color: color.text, backgroundColor: color.live, borderRadius: 6, paddingHorizontal: 7, lineHeight: 22, overflow: 'hidden', fontWeight: '800', letterSpacing: 0.8},
  notice: {flexDirection: 'row', alignItems: 'flex-start', gap: space.s},
});
