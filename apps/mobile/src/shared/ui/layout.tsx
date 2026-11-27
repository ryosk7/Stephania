// Screen, TopBar, Card, List, SectionHeading, StatusView, Sheet: structural
// parts composed by every screen. Drawn by T3Box, not by the platform: no
// native headers, no system alerts, so iOS and Android look the same.
import {Children, createContext, isValidElement, use, type ReactNode} from 'react';
import {KeyboardAvoidingView, Modal, Platform, Pressable, ScrollView, View} from 'react-native';
import {SafeAreaView as RawSafeAreaView} from 'react-native-safe-area-context';
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
 * Bottom sheet drawn by the app (rename, confirmations): same on both
 * platforms, unlike Alert.prompt / system dialogs. `icon` leads the title.
 */
export function Sheet({visible, title, icon, onClose, children}: {visible: boolean; title: string; icon?: IconName; onClose(): void; children: ReactNode}) {
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose} statusBarTranslucent>
      <Pressable className="absolute inset-0 bg-scrim" onPress={onClose} accessibilityLabel="Close" />
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} className="flex-1 justify-end" pointerEvents="box-none">
        <SafeAreaView edges={['bottom']} className="gap-4 rounded-t-sheet border border-border bg-surface px-4 pb-4">
          <View className="mt-2 h-1 w-10 self-center rounded-full bg-tertiary" />
          <View className="flex-row items-center gap-2.5">
            {icon && <Icon name={icon} size={20} color="secondary" />}
            <Title className="flex-1">{title}</Title>
            <IconButton icon="close" label="Close" onPress={onClose} />
          </View>
          {children}
        </SafeAreaView>
      </KeyboardAvoidingView>
    </Modal>
  );
}
