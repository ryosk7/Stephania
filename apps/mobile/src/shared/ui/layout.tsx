// Screen, TopBar, Card, List, SectionHeading, StatusView, Sheet: structural
// parts composed by every screen. Drawn by Stephania, not by the platform: no
// native headers, no system alerts, so iOS and Android look the same.
import {Children, createContext, isValidElement, use, useEffect, useRef, type ReactNode} from 'react';
import {BlurView} from 'expo-blur';
import {Animated, KeyboardAvoidingView, Modal, PanResponder, Platform, Pressable, ScrollView, StyleSheet, View} from 'react-native';
import {SafeAreaInsetsContext, SafeAreaView as RawSafeAreaView} from 'react-native-safe-area-context';
import {withUniwind} from 'uniwind';
import {IconButton} from './buttons';
import {Icon, type IconName} from './icon';
import {Body, Footnote, Headline, Label, Title} from './text';
import {toneBorder, type Tone} from './tone';

const SafeAreaView = withUniwind(RawSafeAreaView);

/**
 * Screen with safe areas on the theme background. `header` is a <TopBar>
 * (or nothing for a full-screen view); `footer` pins the bottom actions
 * (DESIGN.md §3.2).
 */
export function Screen({children, header, footer, scroll = false}: {children: ReactNode; header?: ReactNode; footer?: ReactNode; scroll?: boolean}) {
  return (
    <SafeAreaView edges={['top', 'bottom', 'left', 'right']} className="flex-1 bg-bg">
      {header}
      {scroll
        ? <ScrollView contentContainerClassName="gap-4 p-4" keyboardShouldPersistTaps="handled">{children}</ScrollView>
        : <View className="flex-1 gap-4 p-4">{children}</View>}
      {footer && <View className="gap-3 px-4 py-2">{footer}</View>}
    </SafeAreaView>
  );
}

/** Top bar: round back/close button on the left, optional centred title, trailing icon buttons. */
export function TopBar({title, onBack, backIcon = 'back', trailing}: {title?: string; onBack?: () => void; backIcon?: IconName; trailing?: ReactNode}) {
  return (
    <View className="h-14 flex-row items-center gap-2 px-4">
      <View className="min-w-11 flex-row">{onBack && <IconButton icon={backIcon} label={backIcon === 'close' ? 'Close' : 'Back'} onPress={onBack} />}</View>
      {title ? <Label className="flex-1 text-center" numberOfLines={1}>{title}</Label> : <View className="flex-1" />}
      <View className="min-w-11 flex-row justify-end gap-2">{trailing}</View>
    </View>
  );
}

/** Surface card. Compose with Card.Header / Card.Body. `outlined` rings it in a state colour. */
export function Card({children, outlined}: {children: ReactNode; outlined?: Tone}) {
  return <View className={`gap-3 rounded-card border bg-surface p-4 ${outlined ? toneBorder[outlined] : 'border-border'}`}>{children}</View>;
}
Card.Header = function CardHeader({title, children}: {title: string; children?: ReactNode}) {
  return <View className="flex-row items-center gap-2"><Headline className="flex-1 text-[18px] leading-6" numberOfLines={1}>{title}</Headline>{children}</View>;
};
Card.Body = ({children}: {children: ReactNode}) => <View className="gap-2">{children}</View>;

/**
 * Section heading as an icon (DESIGN.md §3.7): the word is the accessible
 * name, the picture is what people scan. `aside` holds the section's control.
 */
export function SectionHeading({icon, label, aside}: {icon: IconName; label: string; aside?: ReactNode}) {
  return (
    <View className="min-h-7 flex-row items-center gap-3 px-1">
      <View accessible accessibilityRole="header" accessibilityLabel={label}><Icon name={icon} size={18} color="secondary" /></View>
      <View className="flex-1" />
      {aside}
    </View>
  );
}

// List: rows in a bordered card. Rows learn whether they are last from the
// section, so separators need no flags at the call site.
const RowPosition = createContext<{last: boolean}>({last: true});

export function List({children}: {children: ReactNode}) {
  return <View className="gap-6">{children}</View>;
}
List.Section = function ListSection({heading, footer, children}: {heading?: ReactNode; footer?: string; children: ReactNode}) {
  const rows = Children.toArray(children).filter(isValidElement);
  return (
    <View className="gap-2">
      {heading}
      <View className="overflow-hidden rounded-card border border-border bg-surface">
        {rows.map((row, i) => <RowPosition key={row.key ?? i} value={{last: i === rows.length - 1}}>{row}</RowPosition>)}
      </View>
      {footer && <Footnote className="px-1">{footer}</Footnote>}
    </View>
  );
};
List.Row = function ListRow({title, detail, value, leading, accessory, onPress, disabled, selected, accessibilityLabel}: {
  title: string; detail?: string; value?: ReactNode; leading?: ReactNode; accessory?: ReactNode;
  onPress?: () => void; disabled?: boolean; selected?: boolean; accessibilityLabel?: string;
}) {
  const {last} = use(RowPosition);
  const content = (
    <View className={`min-h-14 flex-row items-center gap-3 px-4 py-2 ${last ? '' : 'border-b border-border'}`}>
      {leading}
      <View className="flex-1 gap-0.5">
        <Body tone={disabled ? 'tertiary' : 'primary'}>{title}</Body>
        {detail && <Footnote tone={disabled ? 'tertiary' : 'secondary'}>{detail}</Footnote>}
      </View>
      {typeof value === 'string' ? <Body tone="secondary">{value}</Body> : value}
      {accessory ?? (selected ? <Icon name="check" size={18} strokeWidth={2.5} /> : onPress && !disabled ? <Icon name="chevron" size={16} color="tertiary" /> : null)}
    </View>
  );
  if (!onPress) return <View accessible accessibilityLabel={accessibilityLabel}>{content}</View>;
  return (
    <Pressable accessibilityRole="button" accessibilityState={{disabled, selected}}
      accessibilityLabel={accessibilityLabel ?? [title, detail].filter(Boolean).join(', ')}
      disabled={disabled} onPress={onPress} className="active:bg-pressed">
      {content}
    </Pressable>
  );
};

/** Leading icon on a 36 rounded square; `color` only for state (a muted mic is tertiary). */
export function ListIcon({icon, color}: {icon: IconName; color?: 'text' | 'tertiary' | 'warning'}) {
  return <View className="h-9 w-9 items-center justify-center rounded-[10px] bg-fill"><Icon name={icon} size={18} color={color ?? 'text'} /></View>;
}

/** Centred single-purpose state: setup steps, empty and error states. Actions go in `children`. */
export function StatusView({icon, tone = 'neutral', title, body, children}: {icon: IconName; tone?: Tone; title: string; body?: string; children?: ReactNode}) {
  return (
    <View className="flex-1 items-center justify-center gap-3 px-6">
      <View className={`mb-2 h-16 w-16 items-center justify-center rounded-full border bg-surface ${tone === 'neutral' ? 'border-border' : toneBorder[tone]}`}>
        <Icon name={icon} size={28} tone={tone === 'neutral' ? undefined : tone} />
      </View>
      <Title className="text-center">{title}</Title>
      {body && <Body tone="secondary" className="text-center">{body}</Body>}
      {children && <View className="mt-4 gap-3 self-stretch">{children}</View>}
    </View>
  );
}

/**
 * The one sheet (DESIGN.md §5): device setup, Wi-Fi password, rename,
 * confirmations. Drawn by the app, same on both platforms, unlike
 * Alert.prompt / system dialogs. It floats above the bottom edge with a
 * grabber and no close button: swipe down, tap outside or Android back
 * dismisses it. The head is centred: `art` (any picture) or `icon` (in a
 * round tile), then `title` and an optional `subtitle` line; `children`
 * follow and end with the sheet's action. `bare` drops the head for
 * content that brings its own (the device carousel).
 */
export function Sheet({visible, onClose, title, subtitle, icon, art, bare, children}: {
  visible: boolean; onClose(): void; title?: string; subtitle?: ReactNode; icon?: IconName; art?: ReactNode; bare?: boolean; children: ReactNode;
}) {
  // Context, not useSafeAreaInsets(): stories render without a provider.
  const bottom = use(SafeAreaInsetsContext)?.bottom ?? 0;
  const drag = useRef(new Animated.Value(0)).current;
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const pan = useRef(PanResponder.create({
    onMoveShouldSetPanResponder: (_, g) => g.dy > 6 && Math.abs(g.dy) > Math.abs(g.dx),
    onPanResponderMove: (_, g) => drag.setValue(Math.max(0, g.dy)),
    onPanResponderRelease: (_, g) => {
      if (g.dy > 80 || g.vy > 0.8) { onCloseRef.current(); drag.setValue(0); }
      else Animated.spring(drag, {toValue: 0, useNativeDriver: true}).start();
    },
  })).current;
  // The backdrop fades in; the sheet itself rises from below.
  useEffect(() => {
    if (!visible) return;
    drag.setValue(600);
    Animated.spring(drag, {toValue: 0, useNativeDriver: true, bounciness: 0, speed: 16}).start();
  }, [visible, drag]);
  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose} statusBarTranslucent>
      {/* The screen below stays recognisable: a gentle blur under the scrim. */}
      <BlurView intensity={18} tint="default" style={StyleSheet.absoluteFill} />
      <Pressable className="absolute inset-0 bg-scrim" onPress={onClose} accessibilityLabel="Dismiss" />
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} className="flex-1 justify-end" pointerEvents="box-none">
        {/* 8 pt from the screen edges like the system's floating sheets; the
            content keeps clear of the home indicator inside the sheet. */}
        <View className="px-2 pb-2">
          <Animated.View {...pan.panHandlers} style={{transform: [{translateY: drag}]}}>
            <SheetCard title={title} subtitle={subtitle} icon={icon} art={art} bare={bare} bottom={bottom}>{children}</SheetCard>
          </Animated.View>
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}

/** The sheet's card on its own (what <Sheet> floats over the screen); stories show it inline. */
export function SheetCard({title, subtitle, icon, art, bare, bottom = 0, children}: {
  title?: string; subtitle?: ReactNode; icon?: IconName; art?: ReactNode; bare?: boolean; bottom?: number; children: ReactNode;
}) {
  return (
    <View className="items-center gap-3.5 rounded-sheet border border-border bg-surface px-5 pt-3" style={{paddingBottom: Math.max(24, bottom)}}>
      <View className="h-1 w-10 rounded-full bg-tertiary" accessibilityElementsHidden />
      {!bare && (art ?? (icon && (
        <View className="h-[88px] w-[88px] items-center justify-center rounded-full bg-fill" accessibilityElementsHidden>
          <Icon name={icon} size={40} />
        </View>
      )))}
      {!bare && title && (
        <View className="items-center gap-1">
          <Title className="text-center">{title}</Title>
          {typeof subtitle === 'string' ? <Footnote className="text-center">{subtitle}</Footnote> : subtitle}
        </View>
      )}
      <View className="mt-0.5 gap-3.5 self-stretch">{children}</View>
    </View>
  );
}
