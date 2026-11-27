// Storybook decorator shared by the on-device (.rnstorybook) and web
// (.storybook) Storybooks: every story sits on the theme background with an
// appearance switch, so each component is checked in light and dark.
import type {ComponentType} from 'react';
import {ScrollView, View} from 'react-native';
import {appearanceLabel, appearances} from './appearance';
import {Segmented} from './controls';
import {useAppFonts} from './fonts';
import {setAppearance, useAppearance} from './theme';

function ThemeFrame({Story}: {Story: ComponentType}) {
  const appearance = useAppearance();
  const [fonts, fontError] = useAppFonts();
  if (!fonts && !fontError) return null;
  return (
    <View className="min-h-screen flex-1 bg-bg">
      <View className="w-44 self-end p-3">
        <Segmented label="Appearance" value={appearance} onChange={a => void setAppearance(a)}>
          {appearances.map(a => <Segmented.Item key={a} value={a} label={appearanceLabel[a]} icon={a} />)}
        </Segmented>
      </View>
      <ScrollView contentContainerClassName="gap-4 p-4"><View testID="story"><Story /></View></ScrollView>
    </View>
  );
}

export const themeDecorator = (Story: ComponentType) => <ThemeFrame Story={Story} />;
