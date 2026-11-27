// Button variants as separate components (DESIGN.md §5): explicit variants
// instead of a `kind` flag. Rounded rectangles (radius 14), 52 high; every
// hit area ≥ 44.
import type {ReactNode} from 'react';
import {ActivityIndicator, Pressable, StyleSheet, Text, View} from 'react-native';
import {Icon, type IconName} from './icon';
import {color, radius, size, space, type} from './tokens';

type Base = {title: string; onPress: () => void; disabled?: boolean; busy?: boolean; icon?: IconName; accessibilityHint?: string};

function Filled({title, onPress, disabled, busy, icon, accessibilityHint, bg, fg, border}: Base & {bg: string; fg: string; border?: string}) {
  const inactive = disabled || busy;
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={title} accessibilityHint={accessibilityHint}
      accessibilityState={{disabled: inactive, busy}} disabled={inactive} onPress={onPress}
      style={({pressed}) => [st.filled, {backgroundColor: bg, borderColor: border ?? bg, opacity: inactive ? 0.4 : pressed ? 0.82 : 1, transform: [{scale: pressed ? 0.98 : 1}]}]}>
      {busy ? <ActivityIndicator color={fg} /> : (
        <View style={st.row}>
          {icon && <Icon name={icon} size={18} tint={fg} />}
          <Text style={[type.headline, {color: fg}]}>{title}</Text>
        </View>
      )}
    </Pressable>
  );
}

/** The screen's one main action. */
export const PrimaryButton = (p: Base) => <Filled {...p} bg={color.text} fg={color.onPrimary} />;
/** Stop / end actions: red, never next to a primary of the same size. */
export const DestructiveButton = (p: Base) => <Filled {...p} bg={color.live} fg="#FFFFFF" />;
/** Alternatives to the primary action. */
export const SecondaryButton = (p: Base) => <Filled {...p} bg={color.fill} fg={color.text} border={color.border} />;

/** Quiet way out ("Find it nearby instead"). */
export function TextButton({title, onPress, disabled}: Pick<Base, 'title' | 'onPress' | 'disabled'>) {
  return (
    <Pressable accessibilityRole="button" disabled={disabled} onPress={onPress} hitSlop={8}
      style={({pressed}) => [st.text, {opacity: disabled ? 0.4 : pressed ? 0.6 : 1}]}>
      <Text style={[type.label, {color: color.text, textDecorationLine: 'underline', textDecorationColor: color.textTertiary}]}>{title}</Text>
    </Pressable>
  );
}

/** Round icon-only control (close, back); requires a spoken label. */
export function IconButton({icon, label, onPress, disabled}: {icon: IconName; label: string; onPress: () => void; disabled?: boolean}) {
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={label} disabled={disabled} onPress={onPress} hitSlop={4}
      style={({pressed}) => [st.icon, {opacity: disabled ? 0.4 : pressed ? 0.7 : 1}]}>
      <Icon name={icon} size={20} />
    </Pressable>
  );
}

/** Smaller round control beside a title (rename): 36 drawn, 44 hit area. */
export function InlineIconButton({icon, label, onPress}: {icon: IconName; label: string; onPress: () => void}) {
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={label} onPress={onPress} hitSlop={6}
      style={({pressed}) => [st.inlineIcon, {opacity: pressed ? 0.7 : 1}]}>
      <Icon name={icon} size={16} />
    </Pressable>
  );
}

/** Two buttons side by side. */
export const ButtonRow = ({children}: {children: ReactNode}) => <View style={st.buttonRow}>{children}</View>;

const st = StyleSheet.create({
  filled: {minHeight: size.button, borderRadius: radius.button, borderWidth: 1, alignItems: 'center', justifyContent: 'center', paddingHorizontal: space.xl, flexGrow: 1},
  row: {flexDirection: 'row', alignItems: 'center', gap: space.s},
  text: {minHeight: size.hit, alignItems: 'center', justifyContent: 'center', paddingHorizontal: space.s},
  icon: {width: size.hit, height: size.hit, borderRadius: size.hit / 2, backgroundColor: color.fill, borderWidth: 1, borderColor: color.border, alignItems: 'center', justifyContent: 'center'},
  inlineIcon: {width: 36, height: 36, borderRadius: 18, backgroundColor: color.fill, borderWidth: 1, borderColor: color.border, alignItems: 'center', justifyContent: 'center'},
  buttonRow: {flexDirection: 'row', gap: space.m},
});
