// Input controls drawn by Stephania (same on every platform): Segmented
// (compound, state through context; icon or word per item), Toggle, Field.
import * as Haptics from 'expo-haptics';
import {createContext, use, useEffect, useRef, useState, type ReactNode} from 'react';
import {Animated, Pressable, Text, TextInput, View, type TextInputProps} from 'react-native';
import {Icon, type IconName} from './icon';
import {useColor} from './theme';

type SegmentedState = {value: string; select(v: string): void; locked: boolean};
const SegmentedContext = createContext<SegmentedState | null>(null);

/**
 * Mutually exclusive modes.
 * <Segmented value onChange label><Segmented.Item value label icon /></Segmented>
 * With `icon` an item shows only the icon; `label` is then its spoken name.
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
      <View className="flex-row gap-1 rounded-[14px] border border-border bg-surface p-1" accessibilityRole="radiogroup" accessibilityLabel={label}>{children}</View>
    </SegmentedContext>
  );
}
Segmented.Item = function SegmentedItem({value, label, icon, unavailable}: {value: string; label: string; icon?: IconName; unavailable?: boolean}) {
  const ctx = use(SegmentedContext);
  if (!ctx) throw new Error('Segmented.Item outside Segmented');
  const selected = ctx.value === value;
  const disabled = ctx.locked || unavailable;
  const fg = selected ? 'on-primary' : disabled ? 'tertiary' : 'text';
  return (
    <Pressable accessibilityRole="radio" accessibilityLabel={label} accessibilityState={{selected, disabled}}
      disabled={disabled} onPress={() => ctx.select(value)}
      className={`min-h-10 flex-1 items-center justify-center rounded-[10px] ${selected ? 'bg-text' : ''}`}>
      {icon ? <Icon name={icon} size={20} color={fg} />
        : <Text className={`font-inter-medium text-label ${selected ? 'text-on-primary' : disabled ? 'text-tertiary' : 'text-text'}`}>{label}</Text>}
    </Pressable>
  );
};

/**
 * On/off control drawn by Stephania (not the system switch): a 48×28 track in
 * the text colour when on, so the look is the same on iOS and Android.
 */
export function Toggle({value, onChange, label, disabled}: {value: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean}) {
  const x = useRef(new Animated.Value(value ? 1 : 0)).current;
  useEffect(() => { Animated.timing(x, {toValue: value ? 1 : 0, duration: 150, useNativeDriver: false}).start(); }, [value, x]);
  const on = useColor('text'), off = useColor('track-off'), knobOn = useColor('bg');
  const bg = x.interpolate({inputRange: [0, 1], outputRange: [off, on]});
  const knobBg = x.interpolate({inputRange: [0, 1], outputRange: ['#FFFFFF', knobOn]});
  const left = x.interpolate({inputRange: [0, 1], outputRange: [3, 23]});
  return (
    <Pressable accessibilityRole="switch" accessibilityLabel={label} accessibilityState={{checked: value, disabled}} disabled={disabled}
      onPress={() => { void Haptics.selectionAsync(); onChange(!value); }} hitSlop={8} className={disabled ? 'opacity-40' : ''}>
      <Animated.View style={{width: 48, height: 28, borderRadius: 14, justifyContent: 'center', backgroundColor: bg}}>
        <Animated.View style={{position: 'absolute', top: 3, width: 22, height: 22, borderRadius: 11, left, backgroundColor: knobBg,
          shadowColor: '#000', shadowOpacity: 0.2, shadowRadius: 1.5, shadowOffset: {width: 0, height: 1}, elevation: 1}} />
      </Animated.View>
    </Pressable>
  );
}

/**
 * Text input. `label` is the spoken name; with `icon` the field shows the
 * icon instead of a visible label (DESIGN.md §3.7), otherwise the label sits above.
 */
export function Field({label, icon, secure, ...input}: TextInputProps & {label: string; icon?: IconName; secure?: boolean}) {
  const [focused, setFocused] = useState(false);
  const placeholder = useColor('tertiary'), caret = useColor('text');
  return (
    <View className="gap-1.5">
      {!icon && <Text className="px-1 font-inter text-footnote text-secondary">{label}</Text>}
      <View className={`h-[50px] flex-row items-center gap-3 rounded-control border bg-surface px-4 ${focused ? 'border-secondary' : 'border-border'}`}>
        {icon && <Icon name={icon} size={20} color="secondary" />}
        <TextInput accessibilityLabel={label} placeholderTextColor={placeholder} secureTextEntry={secure}
          autoCapitalize="none" autoCorrect={false} selectionColor={caret} cursorColor={caret} {...input}
          onFocus={e => { setFocused(true); input.onFocus?.(e); }} onBlur={e => { setFocused(false); input.onBlur?.(e); }}
          className="h-full flex-1 font-inter text-body text-text" />
      </View>
    </View>
  );
}
