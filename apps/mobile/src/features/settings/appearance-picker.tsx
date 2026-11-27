// Three preview tiles — system, light, dark — each a miniature screen in
// that theme, with the theme's icon under it (DESIGN.md §3.7). The tile
// colours are fixed: they show each theme, whatever the current one is.
import * as Haptics from 'expo-haptics';
import {Pressable, View} from 'react-native';
import {Icon, appearanceLabel, appearances, setAppearance, useAppearance, type Appearance} from '@/shared/ui';

const look = {
  light: {bg: '#F6F6F7', surface: '#FFFFFF', border: 'rgba(9,9,11,0.09)', text: '#18181B'},
  dark: {bg: '#0B0B0C', surface: '#161618', border: 'rgba(255,255,255,0.08)', text: '#F4F4F5'},
};

function Half({p}: {p: (typeof look)['light']}) {
  return (
    <View className="flex-1 gap-1.5 px-2 py-2.5" style={{backgroundColor: p.bg}}>
      <View className="h-1.5 w-3/5 rounded-full" style={{backgroundColor: p.text}} />
      <View className="h-4 rounded" style={{backgroundColor: p.surface, borderColor: p.border, borderWidth: 1}} />
      <View className="h-4 rounded" style={{backgroundColor: p.surface, borderColor: p.border, borderWidth: 1}} />
    </View>
  );
}

export function AppearancePicker() {
  const current = useAppearance();
  const pick = (a: Appearance) => {
    if (a === current) return;
    void Haptics.selectionAsync();
    void setAppearance(a);
  };
  return (
    <View className="flex-row gap-3" accessibilityRole="radiogroup" accessibilityLabel="Appearance">
      {appearances.map(a => {
        const selected = a === current;
        return (
          <Pressable key={a} accessibilityRole="radio" accessibilityLabel={appearanceLabel[a]} accessibilityState={{selected}}
            onPress={() => pick(a)} className="flex-1 items-center gap-2.5 active:opacity-80">
            <View className={`aspect-[3/4] w-full flex-row overflow-hidden rounded-[14px] ${selected ? 'border-2 border-live' : 'border border-border'}`}>
              {a === 'dark' ? <Half p={look.dark} /> : <Half p={look.light} />}
              {a === 'system' && <Half p={look.dark} />}
              {selected && (
                <View className="absolute right-1.5 top-1.5 h-5 w-5 items-center justify-center rounded-full bg-live">
                  <Icon name="check" size={12} strokeWidth={3} color="white" />
                </View>
              )}
            </View>
            <Icon name={a} size={22} color={selected ? 'text' : 'secondary'} />
          </Pressable>
        );
      })}
    </View>
  );
}
