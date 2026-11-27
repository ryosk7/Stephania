// Settings (DESIGN.md §6): appearance first — follow the system (default),
// light or dark — then licences and the app version.
import Constants from 'expo-constants';
import {router} from 'expo-router';
import {View} from 'react-native';
import {List, ListIcon, Mono, Screen, SectionHeading, TopBar} from '@/shared/ui';
import {AppearancePicker} from './appearance-picker';

export function SettingsScreen() {
  const version = Constants.expoConfig?.version ?? '';
  return (
    <Screen scroll header={<TopBar title="Settings" onBack={() => router.back()} />}>
      <View className="gap-3">
        <SectionHeading icon="light" label="Appearance" />
        <AppearancePicker />
      </View>
      <List.Section heading={<SectionHeading icon="info" label="About" />}>
        <List.Row title="Licences" leading={<ListIcon icon="info" />} onPress={() => router.push('/licences')} />
      </List.Section>
      <Mono tone="tertiary" className="text-center text-[12px]">Stephania Connect {version}</Mono>
    </Screen>
  );
}
