// Input controls: Segmented (compound, state through context), Field.
import * as Haptics from 'expo-haptics';
import {createContext, use, useState, type ReactNode} from 'react';
import {Pressable, StyleSheet, Text, TextInput, View, type TextInputProps} from 'react-native';
import {Caption} from './text';
import {color, radius, size, space, type} from './tokens';

type SegmentedState = {value: string; select(v: string): void; locked: boolean};
const SegmentedContext = createContext<SegmentedState | null>(null);

/**
 * Mutually exclusive modes (Tesla "Off | No A/C | On").
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
      disabled={disabled} onPress={() => ctx.select(value)}
      style={[st.item, selected && st.itemSelected, disabled && !selected && {opacity: 0.35}]}>
      <Text style={[type.headline, {color: selected ? color.onPrimary : color.text, fontSize: 15}]}>{label}</Text>
    </Pressable>
  );
};

/** Filled text input with its label above (Alexa Wi-Fi form). */
export function Field({label, secure, ...input}: TextInputProps & {label: string; secure?: boolean}) {
  const [focused, setFocused] = useState(false);
  return (
    <View style={{gap: space.xs}}>
      <Caption style={{paddingHorizontal: space.xs}}>{label}</Caption>
      <TextInput accessibilityLabel={label} placeholderTextColor={color.textTertiary} secureTextEntry={secure}
        autoCapitalize="none" autoCorrect={false} {...input}
        onFocus={e => { setFocused(true); input.onFocus?.(e); }} onBlur={e => { setFocused(false); input.onBlur?.(e); }}
        style={[type.body, st.field, focused && {borderColor: color.info}]} />
    </View>
  );
}

const st = StyleSheet.create({
  track: {flexDirection: 'row', backgroundColor: color.surfaceRaised, borderRadius: radius.control + 2, padding: 3},
  item: {flex: 1, minHeight: size.hit - 6, borderRadius: radius.control, alignItems: 'center', justifyContent: 'center'},
  itemSelected: {backgroundColor: color.text},
  field: {color: color.text, backgroundColor: color.surface, borderRadius: radius.control, borderWidth: 1, borderColor: 'transparent', paddingHorizontal: space.l, minHeight: size.primaryButton - 4},
});
