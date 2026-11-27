// Screen, TopBar, Card, List, StatusView, Sheet: structural parts composed by
// every screen. Drawn by T3Box, not by the platform: no native headers, no
// system alerts, so iOS and Android look the same.
import {Children, createContext, isValidElement, use, type ReactNode} from 'react';
import {KeyboardAvoidingView, Modal, Platform, Pressable, ScrollView, StyleSheet, View} from 'react-native';
import {SafeAreaView} from 'react-native-safe-area-context';
import {IconButton} from './buttons';
import {Icon, type IconName} from './icon';
import {Body, Footnote, Headline, Label, Overline, Title} from './text';
import {color, radius, size, space, toneColor, type Tone} from './tokens';

/**
 * Dark screen with safe areas. `header` is a <TopBar> (or nothing for a
 * full-screen view); `footer` pins the bottom actions (DESIGN.md §3.2).
 */
export function Screen({children, header, footer, scroll = false}: {children: ReactNode; header?: ReactNode; footer?: ReactNode; scroll?: boolean}) {
  const body = scroll
    ? <ScrollView contentContainerStyle={st.content} keyboardShouldPersistTaps="handled">{children}</ScrollView>
    : <View style={[st.content, {flex: 1}]}>{children}</View>;
  return (
    <SafeAreaView edges={['top', 'bottom', 'left', 'right']} style={st.screen}>
      {header}
      {body}
      {footer && <View style={st.footer}>{footer}</View>}
    </SafeAreaView>
  );
}

/** Top bar: round back/close button on the left, optional centred title and trailing control. */
export function TopBar({title, onBack, backIcon = 'back', trailing}: {title?: string; onBack?: () => void; backIcon?: IconName; trailing?: ReactNode}) {
  return (
    <View style={st.topBar}>
      <View style={st.topSide}>{onBack && <IconButton icon={backIcon} label={backIcon === 'close' ? 'Close' : 'Back'} onPress={onBack} />}</View>
      {title ? <Label style={{flex: 1, textAlign: 'center'}} numberOfLines={1}>{title}</Label> : <View style={{flex: 1}} />}
      <View style={[st.topSide, {alignItems: 'flex-end'}]}>{trailing}</View>
    </View>
  );
}

/** Surface card. Compose with Card.Header / Card.Body. `outlined` rings it in a state colour. */
export function Card({children, outlined}: {children: ReactNode; outlined?: Tone}) {
  return <View style={[st.card, outlined && {borderColor: toneColor[outlined]}]}>{children}</View>;
}
Card.Header = function CardHeader({title, children}: {title: string; children?: ReactNode}) {
  return <View style={st.cardHeader}><Headline style={{flex: 1, fontSize: 18, lineHeight: 24}}>{title}</Headline>{children}</View>;
};
Card.Body = ({children}: {children: ReactNode}) => <View style={{gap: space.s}}>{children}</View>;

// List: rows in a bordered card. Rows learn whether they are last from the
// section, so separators need no flags at the call site.
const RowPosition = createContext<{last: boolean}>({last: true});

export function List({children}: {children: ReactNode}) {
  return <View style={{gap: space.xl}}>{children}</View>;
}
List.Section = function ListSection({title, footer, children}: {title?: string; footer?: string; children: ReactNode}) {
  const rows = Children.toArray(children).filter(isValidElement);
  return (
    <View style={{gap: space.s}}>
      {title && <Overline style={{paddingHorizontal: space.xs}}>{title}</Overline>}
      <View style={st.group}>
        {rows.map((row, i) => <RowPosition key={row.key ?? i} value={{last: i === rows.length - 1}}>{row}</RowPosition>)}
      </View>
      {footer && <Footnote style={{paddingHorizontal: space.xs}}>{footer}</Footnote>}
    </View>
  );
};
List.Row = function ListRow({title, detail, value, leading, accessory, onPress, disabled, selected}: {
  title: string; detail?: string; value?: string; leading?: ReactNode; accessory?: ReactNode;
  onPress?: () => void; disabled?: boolean; selected?: boolean;
}) {
  const {last} = use(RowPosition);
  const content = (
    <View style={[st.row, !last && st.rowSeparator]}>
      {leading}
      <View style={{flex: 1, gap: 2}}>
        <Body tone={disabled ? 'tertiary' : 'primary'}>{title}</Body>
        {detail && <Footnote tone={disabled ? 'tertiary' : 'secondary'}>{detail}</Footnote>}
      </View>
      {value && <Body tone="secondary">{value}</Body>}
      {accessory ?? (selected ? <Icon name="check" size={18} tint={color.text} strokeWidth={2.5} /> : onPress ? <Icon name="chevron" size={16} tint={color.textTertiary} /> : null)}
    </View>
  );
  if (!onPress) return content;
  return (
    <Pressable accessibilityRole="button" accessibilityState={{disabled, selected}} accessibilityLabel={[title, detail, value].filter(Boolean).join(', ')}
      disabled={disabled} onPress={onPress} style={({pressed}) => ({backgroundColor: pressed ? color.surfaceRaised : 'transparent'})}>
      {content}
    </Pressable>
  );
};

/** Leading icon on a 32 rounded square, neutral: colour is reserved for state. */
export function ListIcon({icon}: {icon: IconName}) {
  return <View style={st.listIcon}><Icon name={icon} size={17} /></View>;
}

/** Centred single-purpose state: setup steps, empty and error states. Actions go in `children`. */
export function StatusView({icon, tone = 'neutral', title, body, children}: {icon: IconName; tone?: Tone; title: string; body?: string; children?: ReactNode}) {
  return (
    <View style={st.status}>
      <View style={[st.statusIcon, tone !== 'neutral' && {borderColor: toneColor[tone]}]}>
        <Icon name={icon} size={28} tint={tone === 'neutral' ? color.text : toneColor[tone]} />
      </View>
      <Title style={{textAlign: 'center'}}>{title}</Title>
      {body && <Body tone="secondary" style={{textAlign: 'center'}}>{body}</Body>}
      {children && <View style={{alignSelf: 'stretch', gap: space.m, marginTop: space.l}}>{children}</View>}
    </View>
  );
}

/**
 * Bottom sheet drawn by the app (rename, confirmations): same on both
 * platforms, unlike Alert.prompt / system dialogs.
 */
export function Sheet({visible, title, onClose, children}: {visible: boolean; title: string; onClose(): void; children: ReactNode}) {
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose} statusBarTranslucent>
      <Pressable style={st.scrim} onPress={onClose} accessibilityLabel="Close" />
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} style={st.sheetWrap} pointerEvents="box-none">
        <SafeAreaView edges={['bottom']} style={st.sheet}>
          <View style={st.grabber} />
          <View style={st.sheetHeader}>
            <Title style={{flex: 1}}>{title}</Title>
            <IconButton icon="close" label="Close" onPress={onClose} />
          </View>
          {children}
        </SafeAreaView>
      </KeyboardAvoidingView>
    </Modal>
  );
}

const st = StyleSheet.create({
  screen: {flex: 1, backgroundColor: color.bg},
  content: {padding: size.screenPadding, gap: space.l},
  footer: {paddingHorizontal: size.screenPadding, paddingTop: space.s, paddingBottom: space.s, gap: space.m},
  topBar: {height: 56, flexDirection: 'row', alignItems: 'center', paddingHorizontal: size.screenPadding, gap: space.s},
  topSide: {width: size.hit},
  card: {backgroundColor: color.surface, borderRadius: radius.card, borderWidth: 1, borderColor: color.border, padding: space.l, gap: space.m},
  cardHeader: {flexDirection: 'row', alignItems: 'center', gap: space.s},
  group: {backgroundColor: color.surface, borderRadius: radius.card, borderWidth: 1, borderColor: color.border, overflow: 'hidden'},
  row: {minHeight: 56, flexDirection: 'row', alignItems: 'center', gap: space.m, paddingHorizontal: space.l, paddingVertical: space.s},
  rowSeparator: {borderBottomWidth: 1, borderBottomColor: color.separator},
  listIcon: {width: 32, height: 32, borderRadius: 10, backgroundColor: color.fill, alignItems: 'center', justifyContent: 'center'},
  status: {flex: 1, alignItems: 'center', justifyContent: 'center', gap: space.m, paddingHorizontal: space.xl},
  statusIcon: {width: 64, height: 64, borderRadius: 32, borderWidth: 1, borderColor: color.border, backgroundColor: color.surface, alignItems: 'center', justifyContent: 'center', marginBottom: space.s},
  scrim: {position: 'absolute', left: 0, right: 0, top: 0, bottom: 0, backgroundColor: color.scrim},
  sheetWrap: {flex: 1, justifyContent: 'flex-end'},
  sheet: {backgroundColor: color.surface, borderTopLeftRadius: radius.sheet, borderTopRightRadius: radius.sheet, borderWidth: 1, borderColor: color.border, paddingHorizontal: space.l, paddingBottom: space.l, gap: space.l},
  grabber: {alignSelf: 'center', width: 40, height: 4, borderRadius: 2, backgroundColor: color.textTertiary, marginTop: space.s},
  sheetHeader: {flexDirection: 'row', alignItems: 'center', gap: space.s},
});
