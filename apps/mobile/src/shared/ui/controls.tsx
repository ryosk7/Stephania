// Input controls drawn by T3Box (same on every platform): Segmented
// (compound, state through context), Toggle, Field.
import * as Haptics from 'expo-haptics';
import {createContext, use, useEffect, useRef, useState, type ReactNode} from 'react';
import {Animated, Pressable, StyleSheet, Text, TextInput, View, type TextInputProps} from 'react-native';
import {Footnote} from './text';
import {color, radius, size, space, type} from './tokens';

type SegmentedState = {value: string; select(v: string): void; locked: boolean};
const SegmentedContext = createContext<SegmentedState | null>(null);

/**
 * Mutually exclusive modes.
 * <Segmented value onChange><Segmented.Item value label /></Segmented>
 * `locked` disables every item (e.g. while live) without per-item flags.
 */
export function Segmented<T extends string>({value, onChange, locked = false, label, children}: {
  value: T; onChange: (v: T) => void; locked?: boolean; label: string; children: ReactNode;
}) {
  const select = (v: string) => {
    if (v === value) return;
    void Haptics.selectionAsync();
    onChange(v as T);
  };
  return (
    <SegmentedContext value={{value, select, locked}}>
      <View style={st.track} accessibilityRole="radiogroup" accessibilityLabel={label}>{children}</View>
    </SegmentedContext>
  );
}
Segmented.Item = function SegmentedItem({value, label, unavailable}: {value: string; label: string; unavailable?: boolean}) {
  const ctx = use(SegmentedContext);
  if (!ctx) throw new Error('Segmented.Item outside Segmented');
  const selected = ctx.value === value;
  const disabled = ctx.locked || unavailable;
  return (
    <Pressable accessibilityRole="radio" accessibilityLabel={label} accessibilityState={{selected, disabled}}
      disabled={disabled} onPress={() => ctx.select(value)} style={[st.item, selected && st.itemSelected]}>
      <Text style={[type.label, {color: selected ? color.onPrimary : disabled ? color.textTertiary : color.text}]}>{label}</Text>
    </Pressable>
  );
};

/**
 * On/off control drawn by T3Box (not the system switch): a 48×28 track,
 * light when on with a dark knob, so the look is the same on iOS and Android.
 */
export function Toggle({value, onChange, label, disabled}: {value: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean}) {
  const x = useRef(new Animated.Value(value ? 1 : 0)).current;
  useEffect(() => { Animated.timing(x, {toValue: value ? 1 : 0, duration: 150, useNativeDriver: false}).start(); }, [value, x]);
  const bg = x.interpolate({inputRange: [0, 1], outputRange: ['rgba(255,255,255,0.12)', color.text]});
  const knobBg = x.interpolate({inputRange: [0, 1], outputRange: [color.textSecondary, color.onPrimary]});
  const left = x.interpolate({inputRange: [0, 1], outputRange: [3, 23]});
  return (
    <Pressable accessibilityRole="switch" accessibilityLabel={label} accessibilityState={{checked: value, disabled}} disabled={disabled}
      onPress={() => { void Haptics.selectionAsync(); onChange(!value); }} hitSlop={8} style={{opacity: disabled ? 0.4 : 1}}>
      <Animated.View style={[st.toggle, {backgroundColor: bg}]}>
        <Animated.View style={[st.knob, {left, backgroundColor: knobBg}]} />
      </Animated.View>
    </Pressable>
  );
}

/** Filled text input with its label above. */
export function Field({label, secure, ...input}: TextInputProps & {label: string; secure?: boolean}) {
  const [focused, setFocused] = useState(false);
  return (
    <View style={{gap: 6}}>
      <Footnote style={{paddingHorizontal: space.xs}}>{label}</Footnote>
      <TextInput accessibilityLabel={label} placeholderTextColor={color.textTertiary} secureTextEntry={secure}
        autoCapitalize="none" autoCorrect={false} selectionColor={color.text} {...input}
        onFocus={e => { setFocused(true); input.onFocus?.(e); }} onBlur={e => { setFocused(false); input.onBlur?.(e); }}
        style={[type.body, st.field, focused && {borderColor: color.textSecondary}]} />
    </View>
  );
}

const st = StyleSheet.create({
  track: {flexDirection: 'row', gap: 4, backgroundColor: color.surface, borderWidth: 1, borderColor: color.border, borderRadius: radius.control + 2, padding: 4},
  item: {flex: 1, minHeight: size.hit - 4, borderRadius: radius.control - 2, alignItems: 'center', justifyContent: 'center'},
  itemSelected: {backgroundColor: color.text},
  toggle: {width: 48, height: 28, borderRadius: 14, justifyContent: 'center'},
  knob: {position: 'absolute', top: 3, width: 22, height: 22, borderRadius: 11},
  field: {color: color.text, backgroundColor: color.surface, borderRadius: radius.control, borderWidth: 1, borderColor: color.border, paddingHorizontal: space.l, height: 50},
});
