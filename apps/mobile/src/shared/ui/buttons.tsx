// Buttons as separate components (DESIGN.md §5): explicit variants instead
// of a `kind` flag. 52 high, radius 14; every hit area ≥ 44. Text buttons
// carry an icon plus one word; icon-only buttons require a spoken label.
import * as Haptics from 'expo-haptics';
import {useEffect, useState, type ReactNode} from 'react';
import {ActivityIndicator, Pressable, Text, View} from 'react-native';
import {Icon, type IconName} from './icon';
import {useColor, type ColorToken} from './theme';

type Base = {title: string; onPress: () => void; disabled?: boolean; busy?: boolean; icon?: IconName; accessibilityLabel?: string; accessibilityHint?: string};

function Filled({title, onPress, disabled, busy, icon, accessibilityLabel, accessibilityHint, box, fg}: Base & {box: string; fg: ColorToken}) {
  const inactive = disabled || busy;
  const spinner = useColor(fg);
  // A forward arrow follows the word ("Join →"); every other icon leads.
  const trailing = icon === 'arrow';
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={accessibilityLabel ?? title} accessibilityHint={accessibilityHint}
      accessibilityState={{disabled: inactive, busy}} disabled={inactive} onPress={onPress}
      className={`min-h-[52px] grow flex-row items-center justify-center gap-2 rounded-button border px-6 active:opacity-80 ${box} ${inactive ? 'opacity-40' : ''}`}>
      {busy ? <ActivityIndicator color={spinner} /> : (
        <>
          {icon && !trailing && <Icon name={icon} size={18} color={fg} />}
          <Text className={`font-inter-semibold text-headline ${fg === 'white' ? 'text-white' : fg === 'on-primary' ? 'text-on-primary' : 'text-text'}`}>{title}</Text>
          {icon && trailing && <Icon name={icon} size={18} color={fg} />}
        </>
      )}
    </Pressable>
  );
}

/** The screen's one main action. */
export const PrimaryButton = (p: Base) => <Filled {...p} box="bg-text border-text" fg="on-primary" />;
/** Stop / end actions: red, never next to a primary of the same size. */
export const DestructiveButton = (p: Base) => <Filled {...p} box="bg-live border-live" fg="white" />;
/** Alternatives to the primary action. */
export const SecondaryButton = (p: Base) => <Filled {...p} box="bg-fill border-border" fg="text" />;

/** Quiet way out, in words (used where an icon would be ambiguous). */
export function TextButton({title, onPress, disabled}: Pick<Base, 'title' | 'onPress' | 'disabled'>) {
  return (
    <Pressable accessibilityRole="button" disabled={disabled} onPress={onPress} hitSlop={8}
      className={`min-h-11 items-center justify-center px-2 active:opacity-60 ${disabled ? 'opacity-40' : ''}`}>
      <Text className="font-inter-medium text-label text-text underline">{title}</Text>
    </Pressable>
  );
}

const iconBox = {
  round: 'h-11 w-11 rounded-full',
  inline: 'h-9 w-9 rounded-full',
  square: 'h-[52px] w-[52px] rounded-button',
};

/**
 * Icon-only control. The label is spoken by screen readers and shown on
 * long press, so the meaning is never only in the picture (DESIGN.md §3.7).
 * round: 44 (back, close, toolbar); inline: 36 beside a title (44 hit area);
 * square: 52, next to a text button of the same height.
 */
export function IconButton({icon, label, onPress, disabled, shape = 'round'}: {
  icon: IconName; label: string; onPress: () => void; disabled?: boolean; shape?: keyof typeof iconBox;
}) {
  const [hint, setHint] = useState(false);
  useEffect(() => {
    if (!hint) return;
    const t = setTimeout(() => setHint(false), 1600);
    return () => clearTimeout(t);
  }, [hint]);
  return (
    <View>
      <Pressable accessibilityRole="button" accessibilityLabel={label} disabled={disabled} onPress={onPress}
        onLongPress={() => { void Haptics.selectionAsync(); setHint(true); }} hitSlop={shape === 'inline' ? 6 : 4}
        className={`items-center justify-center border border-border bg-fill active:opacity-70 ${iconBox[shape]} ${disabled ? 'opacity-40' : ''}`}>
        <Icon name={icon} size={shape === 'inline' ? 16 : shape === 'square' ? 22 : 20} />
      </Pressable>
      {hint && (
        <View pointerEvents="none" importantForAccessibility="no-hide-descendants" accessibilityElementsHidden
          className="absolute top-full z-10 mt-1.5 self-center rounded-lg bg-text px-2.5 py-1">
          <Text numberOfLines={1} className="font-inter-medium text-footnote text-on-primary">{label}</Text>
        </View>
      )}
    </View>
  );
}

/** Buttons side by side. */
export const ButtonRow = ({children}: {children: ReactNode}) => <View className="flex-row gap-3">{children}</View>;
