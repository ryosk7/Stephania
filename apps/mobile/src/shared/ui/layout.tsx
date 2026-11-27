// Screen, Card, List, StatusView: structural parts composed by every screen.
import {Children, createContext, isValidElement, use, type ReactNode} from 'react';
import {Pressable, ScrollView, StyleSheet, View} from 'react-native';
import {SafeAreaView} from 'react-native-safe-area-context';
import {Symbol, symbols, type SymbolName} from './symbol';
import {Body, Caption, Footnote, Headline, Title} from './text';
import {color, radius, size, space, toneColor, type Tone} from './tokens';

/** Dark screen with safe areas; `footer` pins the bottom actions (DESIGN.md §3.2). */
export function Screen({children, footer, scroll = false}: {children: ReactNode; footer?: ReactNode; scroll?: boolean}) {
  const body = scroll
    ? <ScrollView contentContainerStyle={st.content} keyboardShouldPersistTaps="handled">{children}</ScrollView>
    : <View style={[st.content, {flex: 1}]}>{children}</View>;
  return (
    <SafeAreaView edges={['bottom', 'left', 'right']} style={st.screen}>
      {body}
      {footer && <View style={st.footer}>{footer}</View>}
    </SafeAreaView>
  );
}

/** Full-screen variant without a navigation header (camera): safe on all edges. */
export function ImmersiveScreen({children, footer}: {children: ReactNode; footer?: ReactNode}) {
  return (
    <SafeAreaView edges={['top', 'bottom', 'left', 'right']} style={st.screen}>
      <View style={[st.content, {flex: 1}]}>{children}</View>
      {footer && <View style={st.footer}>{footer}</View>}
    </SafeAreaView>
  );
}

/** Surface with radius 16. Compose with Card.Header / Card.Body. */
export function Card({children, outlined}: {children: ReactNode; outlined?: Tone}) {
  return <View style={[st.card, outlined && {borderWidth: 2, borderColor: toneColor[outlined]}]}>{children}</View>;
}
Card.Header = function CardHeader({title, children}: {title: string; children?: ReactNode}) {
  return <View style={st.cardHeader}><Headline style={{flex: 1}}>{title}</Headline>{children}</View>;
};
Card.Body = ({children}: {children: ReactNode}) => <View style={{gap: space.s}}>{children}</View>;

// List: inset grouped rows. Rows learn whether they are last from the
// section, so separators need no flags at the call site.
const RowPosition = createContext<{last: boolean}>({last: true});

export function List({children}: {children: ReactNode}) {
  return <View style={{gap: space.xl}}>{children}</View>;
}
List.Section = function ListSection({title, footer, children}: {title?: string; footer?: string; children: ReactNode}) {
  const rows = Children.toArray(children).filter(isValidElement);
  return (
    <View style={{gap: space.s}}>
      {title && <Caption style={st.sectionTitle}>{title.toUpperCase()}</Caption>}
      <View style={st.group}>
        {rows.map((row, i) => <RowPosition key={row.key ?? i} value={{last: i === rows.length - 1}}>{row}</RowPosition>)}
      </View>
      {footer && <Footnote style={{paddingHorizontal: space.l}}>{footer}</Footnote>}
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
        <Body>{title}</Body>
        {detail && <Footnote>{detail}</Footnote>}
      </View>
      {value && <Body tone="secondary">{value}</Body>}
      {accessory ?? (selected ? <Symbol name={symbols.check} size={20} tone="info" /> : onPress ? <Symbol name={symbols.chevron} size={14} tint={color.textTertiary} /> : null)}
    </View>
  );
  if (!onPress) return content;
  return (
    <Pressable accessibilityRole="button" accessibilityState={{disabled, selected}} accessibilityLabel={[title, detail, value].filter(Boolean).join(', ')}
      disabled={disabled} onPress={onPress} style={({pressed}) => ({opacity: disabled ? 0.4 : 1, backgroundColor: pressed ? color.surfaceRaised : 'transparent'})}>
      {content}
    </Pressable>
  );
};

/** Centred single-purpose state: setup steps, empty and error states. Actions go in `children`. */
export function StatusView({symbol, tone = 'neutral', title, body, children}: {symbol: SymbolName; tone?: Tone; title: string; body?: string; children?: ReactNode}) {
  return (
    <View style={st.status}>
      <Symbol name={symbol} size={44} tint={tone === 'neutral' ? color.text : toneColor[tone]} />
      <Title style={{textAlign: 'center'}}>{title}</Title>
      {body && <Body tone="secondary" style={{textAlign: 'center'}}>{body}</Body>}
      {children && <View style={{alignSelf: 'stretch', gap: space.m, marginTop: space.l}}>{children}</View>}
    </View>
  );
}

const st = StyleSheet.create({
  screen: {flex: 1, backgroundColor: color.bg},
  content: {padding: size.screenPadding, gap: space.l},
  footer: {paddingHorizontal: size.screenPadding, paddingTop: space.s, paddingBottom: space.s, gap: space.m},
  card: {backgroundColor: color.surface, borderRadius: radius.card, padding: space.l, gap: space.m},
  cardHeader: {flexDirection: 'row', alignItems: 'center', gap: space.s},
  sectionTitle: {paddingHorizontal: space.l, letterSpacing: 0.4},
  group: {backgroundColor: color.surface, borderRadius: radius.card, overflow: 'hidden'},
  row: {minHeight: size.hit + 8, flexDirection: 'row', alignItems: 'center', gap: space.m, paddingHorizontal: space.l, paddingVertical: space.m},
  rowSeparator: {borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: color.separator},
  status: {flex: 1, alignItems: 'center', justifyContent: 'center', gap: space.m, paddingHorizontal: space.xl},
});
