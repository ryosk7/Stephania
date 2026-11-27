// Button variants as separate components (DESIGN.md §5, composition:
// explicit variants instead of a `kind` flag). Every hit area ≥ 44 pt.
import type {ReactNode} from 'react';
import {ActivityIndicator, Pressable, StyleSheet, Text, View} from 'react-native';
import {Symbol, type SymbolName} from './symbol';
import {color, radius, size, space, type} from './tokens';

type Base = {title: string; onPress: () => void; disabled?: boolean; busy?: boolean; symbol?: SymbolName; accessibilityHint?: string};

function Filled({title, onPress, disabled, busy, symbol, accessibilityHint, bg, fg}: Base & {bg: string; fg: string}) {
  const inactive = disabled || busy;
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={title} accessibilityHint={accessibilityHint}
      accessibilityState={{disabled: inactive, busy}} disabled={inactive} onPress={onPress}
      style={({pressed}) => [st.filled, {backgroundColor: bg, opacity: inactive ? 0.4 : pressed ? 0.8 : 1}]}>
      {busy ? <ActivityIndicator color={fg} /> : (
        <View style={st.row}>
          {symbol && <Symbol name={symbol} size={18} tint={fg} />}
          <Text style={[type.headline, {color: fg, letterSpacing: -0.2}]}>{title}</Text>
        </View>
      )}
    </Pressable>
  );
}

/** The screen's one main action: white pill on the dark surface. */
export const PrimaryButton = (p: Base) => <Filled {...p} bg={color.text} fg={color.onPrimary} />;
/** Stop / end actions: red, never next to a primary of the same size. */
export const DestructiveButton = (p: Base) => <Filled {...p} bg={color.live} fg={color.text} />;
/** Alternatives to the primary action. */
export const SecondaryButton = (p: Base) => <Filled {...p} bg={color.fill} fg={color.text} />;

/** Quiet way out ("Find it nearby instead"). */
export function TextButton({title, onPress, disabled}: Pick<Base, 'title' | 'onPress' | 'disabled'>) {
  return (
    <Pressable accessibilityRole="button" disabled={disabled} onPress={onPress} hitSlop={8}
      style={({pressed}) => [st.text, {opacity: disabled ? 0.4 : pressed ? 0.6 : 1}]}>
      <Text style={[type.body, {color: color.info}]}>{title}</Text>
    </Pressable>
  );
}

/** Round icon-only control (close, flip); requires a spoken label. */
export function IconButton({symbol, label, onPress, disabled}: {symbol: SymbolName; label: string; onPress: () => void; disabled?: boolean}) {
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={label} disabled={disabled} onPress={onPress}
      style={({pressed}) => [st.icon, {opacity: disabled ? 0.4 : pressed ? 0.7 : 1}]}>
      <Symbol name={symbol} size={17} />
    </Pressable>
  );
}

/** Smaller round control beside a title (rename); 36 pt drawn, 44 pt hit area. */
export function InlineIconButton({symbol, label, onPress}: {symbol: SymbolName; label: string; onPress: () => void}) {
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={label} onPress={onPress} hitSlop={4}
      style={({pressed}) => [st.inlineIcon, {opacity: pressed ? 0.7 : 1}]}>
      <Symbol name={symbol} size={15} />
    </Pressable>
  );
}

/** Two buttons side by side (Alexa "Change network | Continue"). */
export const ButtonRow = ({children}: {children: ReactNode}) => <View style={st.buttonRow}>{children}</View>;

const st = StyleSheet.create({
  filled: {minHeight: size.primaryButton, borderRadius: radius.pill, alignItems: 'center', justifyContent: 'center', paddingHorizontal: space.xl, flexGrow: 1},
  row: {flexDirection: 'row', alignItems: 'center', gap: space.s},
  text: {minHeight: size.hit, alignItems: 'center', justifyContent: 'center', paddingHorizontal: space.s},
  icon: {width: size.hit, height: size.hit, borderRadius: size.hit / 2, backgroundColor: color.fill, alignItems: 'center', justifyContent: 'center'},
  inlineIcon: {width: 36, height: 36, borderRadius: 18, backgroundColor: color.fill, alignItems: 'center', justifyContent: 'center'},
  buttonRow: {flexDirection: 'row', gap: space.m},
});
