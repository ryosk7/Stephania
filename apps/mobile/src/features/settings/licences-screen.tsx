// Third-party licences of everything that ships in the app, with the
// licence text each package carries (scripts/notices.mjs; docs/LICENSING.md).
import {router} from 'expo-router';
import {useState} from 'react';
import {FlatList, Pressable, View} from 'react-native';
import {Body, Footnote, Icon, Mono, Screen, Sheet, TopBar} from '@/shared/ui';
import notices from './licences.json';

type Package = (typeof notices.packages)[number];
const texts = notices.texts as Record<string, string>;

export function LicencesScreen() {
  const [open, setOpen] = useState<Package | null>(null);
  return (
    <Screen header={<TopBar title="Licences" onBack={() => router.back()} />}>
      <FlatList data={notices.packages} keyExtractor={p => `${p.name}@${p.version}`}
        className="-mx-4 -mt-4" contentContainerClassName="px-4 pb-4"
        renderItem={({item: p}) => (
          <Pressable accessibilityRole="button" accessibilityLabel={`${p.name} ${p.version}, ${p.license}`} disabled={!p.text}
            onPress={() => setOpen(p)} className="min-h-14 flex-row items-center gap-3 border-b border-border py-2 active:bg-pressed">
            <View className="flex-1 gap-0.5">
              <Body numberOfLines={1}>{p.name}</Body>
              <Footnote>{p.license} · {p.version}</Footnote>
            </View>
            {p.text && <Icon name="chevron" size={16} color="tertiary" />}
          </Pressable>
        )} />
      <Sheet visible={open !== null} title={open?.name ?? ''} icon="info" onClose={() => setOpen(null)}>
        <View className="max-h-[480px]">
          <FlatList data={open?.text ? texts[open.text]!.split('\n\n') : []} keyExtractor={(_, i) => String(i)}
            renderItem={({item}) => <Mono tone="secondary" className="mb-3 text-[11px] leading-4">{item}</Mono>} />
        </View>
      </Sheet>
    </Screen>
  );
}
